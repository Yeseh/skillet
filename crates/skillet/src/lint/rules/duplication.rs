//! Rule: `duplication` — warns when near-verbatim passages are shared across
//! compiled skill outputs, suggesting extraction into a fragment.
//!
//! Uses MinHash + LSH for efficient candidate selection instead of O(n²)
//! exhaustive comparison.  MinHash signatures are cached in the lockfile and
//! reused when a skill's compiled output is unchanged.

use crate::lockfile::Lockfile;
use crate::workspace::SkillSource;

use super::{Diagnostic, Severity};

// ── Detection constants ───────────────────────────────────────────────────────

/// Minimum number of consecutive sentences that must match to trigger a warning.
const MIN_SENTENCES: usize = 3;
/// Word-overlap ratio threshold (>80 %).
const OVERLAP_THRESHOLD: f64 = 0.80;

// ── MinHash constants ─────────────────────────────────────────────────────────

/// Number of hash functions for MinHash signatures.
const NHASH: usize = 128;
/// Number of LSH bands.
const BANDS: usize = 16;
/// Rows per band (NHASH / BANDS).
const ROWS: usize = 8;

// ── Public interface ──────────────────────────────────────────────────────────

/// Checks all compiled `SKILL.md` outputs for near-verbatim shared passages.
///
/// Returns `(diagnostics, updated_signatures)` where `updated_signatures`
/// holds `(skill_name, minhash)` entries that were recomputed and should be
/// written back to the lockfile.
pub fn check(
    all_sources: &[SkillSource],
    lockfile: &Lockfile,
) -> (Vec<Diagnostic>, Vec<(String, Vec<u64>)>) {
    // Collect (skill_name, text, minhash_opt) for skills with a compiled output.
    struct SkillData<'a> {
        name: &'a str,
        text: String,
        sig: Vec<u64>,
        sig_was_cached: bool,
    }

    let mut skill_data: Vec<SkillData> = Vec::new();
    let mut updated_sigs: Vec<(String, Vec<u64>)> = Vec::new();

    for src in all_sources {
        let path = src.skill_out_dir.join("SKILL.md");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };

        // Try to load cached signature from lockfile.
        let cached = lockfile
            .skills
            .get(src.name.as_str())
            .filter(|e| !e.minhash.is_empty())
            .map(|e| e.minhash.clone());

        let (sig, was_cached) = if let Some(sig) = cached {
            (sig, true)
        } else {
            let sig = compute_minhash(&text);
            (sig, false)
        };

        skill_data.push(SkillData {
            name: &src.name,
            text,
            sig,
            sig_was_cached: was_cached,
        });
    }

    // Collect new signatures to write back.
    for sd in &skill_data {
        if !sd.sig_was_cached {
            updated_sigs.push((sd.name.to_string(), sd.sig.clone()));
        }
    }

    if skill_data.len() < 2 {
        return (vec![], updated_sigs);
    }

    // ── LSH candidate selection ───────────────────────────────────────────────

    let signatures: Vec<(&str, &[u64])> = skill_data
        .iter()
        .map(|sd| (sd.name, sd.sig.as_slice()))
        .collect();

    let candidate_pairs = lsh_candidates(&signatures);

    if candidate_pairs.is_empty() {
        return (vec![], updated_sigs);
    }

    // ── Full window comparison for candidates ─────────────────────────────────

    let skill_windows: Vec<(&str, Vec<String>)> = skill_data
        .iter()
        .map(|sd| {
            let sents = sentences(&sd.text);
            let wins: Vec<String> = sents.windows(MIN_SENTENCES).map(|w| w.join(" ")).collect();
            (sd.name, wins)
        })
        .collect();

    let mut raw_matches: Vec<(String, &str, &str)> = Vec::new();

    for (i, j) in candidate_pairs {
        let (name_a, wins_a) = &skill_windows[i];
        let (name_b, wins_b) = &skill_windows[j];

        for wa in wins_a {
            for wb in wins_b {
                if overlap_ratio(wa, wb) > OVERLAP_THRESHOLD {
                    let passage = if wa.len() >= wb.len() {
                        wa.clone()
                    } else {
                        wb.clone()
                    };
                    raw_matches.push((passage, name_a, name_b));
                }
            }
        }
    }

    if raw_matches.is_empty() {
        return (vec![], updated_sigs);
    }

    // ── Cluster overlapping matches ───────────────────────────────────────────

    struct Cluster {
        passage: String,
        skills: std::collections::BTreeSet<String>,
    }

    let mut clusters: Vec<Cluster> = Vec::new();

    for (passage, skill_a, skill_b) in raw_matches {
        let mut merged = false;
        for cluster in &mut clusters {
            if overlap_ratio(&cluster.passage, &passage) > 0.50 {
                if passage.len() > cluster.passage.len() {
                    cluster.passage = passage.clone();
                }
                cluster.skills.insert(skill_a.to_string());
                cluster.skills.insert(skill_b.to_string());
                merged = true;
                break;
            }
        }
        if !merged {
            let mut skills = std::collections::BTreeSet::new();
            skills.insert(skill_a.to_string());
            skills.insert(skill_b.to_string());
            clusters.push(Cluster { passage, skills });
        }
    }

    // ── Emit diagnostics ──────────────────────────────────────────────────────

    let mut diags: Vec<Diagnostic> = clusters
        .into_iter()
        .filter(|c| c.skills.len() >= 2)
        .map(|c| {
            let affected_skills: Vec<String> = c.skills.into_iter().collect();
            let excerpt: String = c.passage.chars().take(120).collect();
            let affected = affected_skills.join(", ");
            let msg = format!(
                "passage shared across skills [{affected}]: \"{excerpt}…\" — consider extracting to a fragment"
            );
            Diagnostic {
                rule: "duplication".to_string(),
                severity: Severity::Warning,
                skill: "<workspace>".to_string(),
                message: msg,
                path: None,
                line: None,
                col: None,
                duplicated_text: Some(c.passage),
                affected_skills: Some(affected_skills),
            }
        })
        .collect();

    diags.sort_by(|a, b| a.message.cmp(&b.message));
    (diags, updated_sigs)
}

// ── MinHash ───────────────────────────────────────────────────────────────────

/// Computes a MinHash signature (NHASH × u64) for the sentence-window shingles
/// of `text`.
fn compute_minhash(text: &str) -> Vec<u64> {
    let sents = sentences(text);
    if sents.len() < MIN_SENTENCES {
        return vec![u64::MAX; NHASH];
    }
    let shingles: Vec<String> = sents.windows(MIN_SENTENCES).map(|w| w.join(" ")).collect();

    let mut sig = vec![u64::MAX; NHASH];
    for shingle in &shingles {
        for (i, slot) in sig.iter_mut().enumerate() {
            let h = hash_with_seed(shingle, i as u64);
            if h < *slot {
                *slot = h;
            }
        }
    }
    sig
}

/// Hashes `s` mixed with `seed` using a deterministic 64-bit hash.
fn hash_with_seed(s: &str, seed: u64) -> u64 {
    // FNV-1a inspired: XOR seed into initial state.
    const FNV_OFFSET: u64 = 14_695_981_039_346_656_037;
    const FNV_PRIME: u64 = 1_099_511_628_211;
    let mut hash = FNV_OFFSET ^ seed.wrapping_mul(FNV_PRIME);
    for byte in s.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    // Mix seed again to differentiate hash functions.
    hash ^= seed;
    hash = hash.wrapping_mul(6_364_136_223_846_793_005_u64);
    hash
}

// ── LSH ───────────────────────────────────────────────────────────────────────

/// Returns the set of candidate pairs (i, j) identified by LSH banding.
fn lsh_candidates(signatures: &[(&str, &[u64])]) -> Vec<(usize, usize)> {
    let mut candidate_set = std::collections::HashSet::new();

    for band in 0..BANDS {
        let start = band * ROWS;
        let end = start + ROWS;
        let mut buckets: std::collections::HashMap<Vec<u64>, Vec<usize>> =
            std::collections::HashMap::new();

        for (idx, (_name, sig)) in signatures.iter().enumerate() {
            let band_sig = sig[start..end].to_vec();
            buckets.entry(band_sig).or_default().push(idx);
        }

        for indices in buckets.values() {
            if indices.len() >= 2 {
                for a in 0..indices.len() {
                    for b in a + 1..indices.len() {
                        let pair = (indices[a].min(indices[b]), indices[a].max(indices[b]));
                        candidate_set.insert(pair);
                    }
                }
            }
        }
    }

    candidate_set.into_iter().collect()
}

// ── Sentence helpers (shared with existing detection logic) ───────────────────

fn sentences(text: &str) -> Vec<String> {
    let body = strip_frontmatter(text);
    let cleaned: String = body
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("```"))
        .collect::<Vec<_>>()
        .join(" ");

    let mut out = Vec::new();
    let mut buf = String::new();
    for ch in cleaned.chars() {
        buf.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            let s = normalise(&buf);
            if !s.is_empty() {
                out.push(s);
            }
            buf.clear();
        }
    }
    let s = normalise(&buf);
    if !s.is_empty() {
        out.push(s);
    }
    out
}

fn strip_frontmatter(text: &str) -> &str {
    let t = text.trim_start();
    if !t.starts_with("---") {
        return text;
    }
    let after_open = match t.find('\n') {
        Some(i) => &t[i + 1..],
        None => return text,
    };
    for (i, line) in after_open.lines().enumerate() {
        if line.trim() == "---" {
            let offset: usize = after_open.lines().take(i + 1).map(|l| l.len() + 1).sum();
            return &after_open[offset..];
        }
    }
    text
}

fn normalise(s: &str) -> String {
    s.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn overlap_ratio(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    let wa: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let wb: std::collections::HashSet<&str> = b.split_whitespace().collect();
    let intersection = wa.intersection(&wb).count();
    let union = wa.union(&wb).count();
    if union == 0 {
        0.0
    } else {
        intersection as f64 / union as f64
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::Lockfile;
    use crate::workspace::SkillSource;
    use std::fs;
    use tempfile::TempDir;

    fn make_source(root: &std::path::Path, name: &str, skill_md: &str) -> SkillSource {
        let skill_out = root.join("skills").join(name);
        fs::create_dir_all(&skill_out).unwrap();
        fs::write(skill_out.join("SKILL.md"), skill_md).unwrap();
        let skill_src = root.join("src/skills").join(name);
        fs::create_dir_all(&skill_src).unwrap();
        SkillSource {
            name: name.to_string(),
            source_path: skill_src.join(format!("{name}.pan")),
            skill_dir: skill_src,
            skill_out_dir: skill_out,
        }
    }

    const SHARED: &str = "First sentence here. Second sentence here. Third sentence here.";

    #[test]
    fn detects_cross_skill_duplication() {
        let tmp = TempDir::new().unwrap();
        let a = make_source(tmp.path(), "alpha", &format!("# Alpha\n\n{SHARED}\n"));
        let b = make_source(tmp.path(), "beta", &format!("# Beta\n\n{SHARED}\n"));
        let (diags, _sigs) = check(&[a, b], &Lockfile::default());
        assert!(
            diags.iter().any(|d| d.rule == "duplication"
                && d.severity == Severity::Warning
                && d.affected_skills
                    .as_ref()
                    .map(|v| v.contains(&"alpha".to_string()) && v.contains(&"beta".to_string()))
                    .unwrap_or(false)),
            "expected duplication warning, got: {diags:?}"
        );
    }

    #[test]
    fn no_warning_for_single_skill_repetition() {
        let tmp = TempDir::new().unwrap();
        let a = make_source(
            tmp.path(),
            "solo",
            &format!("# Solo\n\n{SHARED}\n\n{SHARED}\n"),
        );
        let (diags, _) = check(&[a], &Lockfile::default());
        assert!(diags.is_empty());
    }

    #[test]
    fn no_warning_when_passage_unique_per_skill() {
        let tmp = TempDir::new().unwrap();
        let a = make_source(
            tmp.path(),
            "alpha",
            "# Alpha\n\nOnly in alpha. Unique text here. Nothing shared with beta.",
        );
        let b = make_source(
            tmp.path(),
            "beta",
            "# Beta\n\nOnly in beta. Different content. Completely separate.",
        );
        let (diags, _) = check(&[a, b], &Lockfile::default());
        assert!(diags.is_empty());
    }

    #[test]
    fn overlapping_windows_collapsed_to_single_diagnostic() {
        let tmp = TempDir::new().unwrap();
        let content = "Sentence one here. Sentence two here. Sentence three here. \
                       Sentence four here. Sentence five here.";
        let a = make_source(tmp.path(), "alpha", content);
        let b = make_source(tmp.path(), "beta", content);
        let (diags, _) = check(&[a, b], &Lockfile::default());
        assert_eq!(
            diags.len(),
            1,
            "overlapping windows should collapse to one diagnostic, got: {diags:?}"
        );
    }

    #[test]
    fn three_skills_sharing_same_passage_emit_one_diagnostic() {
        let tmp = TempDir::new().unwrap();
        let a = make_source(tmp.path(), "alpha", &format!("# A\n\n{SHARED}\n"));
        let b = make_source(tmp.path(), "beta", &format!("# B\n\n{SHARED}\n"));
        let c = make_source(tmp.path(), "gamma", &format!("# C\n\n{SHARED}\n"));
        let (diags, _) = check(&[a, b, c], &Lockfile::default());
        assert_eq!(
            diags.len(),
            1,
            "three skills sharing same passage should produce one diagnostic, got: {diags:?}"
        );
        let d = &diags[0];
        let skills = d.affected_skills.as_ref().unwrap();
        assert!(skills.contains(&"alpha".to_string()));
        assert!(skills.contains(&"beta".to_string()));
        assert!(skills.contains(&"gamma".to_string()));
    }

    #[test]
    fn returns_updated_signatures_for_uncached_skills() {
        let tmp = TempDir::new().unwrap();
        let a = make_source(tmp.path(), "alpha", &format!("# Alpha\n\n{SHARED}\n"));
        let b = make_source(tmp.path(), "beta", &format!("# Beta\n\n{SHARED}\n"));
        let (_diags, sigs) = check(&[a, b], &Lockfile::default());
        // Both skills should have signatures to cache.
        assert_eq!(sigs.len(), 2);
        assert!(sigs.iter().any(|(name, _)| name == "alpha"));
        assert!(sigs.iter().any(|(name, _)| name == "beta"));
    }

    #[test]
    fn reuses_cached_signatures_from_lockfile() {
        let tmp = TempDir::new().unwrap();
        let a = make_source(tmp.path(), "alpha", &format!("# Alpha\n\n{SHARED}\n"));
        let b = make_source(tmp.path(), "beta", &format!("# Beta\n\n{SHARED}\n"));

        // First run — compute and cache.
        let (_diags, sigs) = check(&[a, b], &Lockfile::default());

        // Build a lockfile with cached signatures.
        let mut lf = Lockfile::default();
        for (name, sig) in sigs {
            lf.skills.insert(
                name.clone(),
                crate::lockfile::SkillEntry {
                    source_hash: String::new(),
                    compiled_hash: String::new(),
                    discovery_tokens: 0,
                    activation_tokens: 0,
                    transitive_tokens: 0,
                    fragments_used: vec![],
                    refs: Default::default(),
                    minhash: sig,
                },
            );
        }

        // Recreate sources — need new tempdir paths but same lockfile.
        let tmp2 = TempDir::new().unwrap();
        let a2 = make_source(tmp2.path(), "alpha", &format!("# Alpha\n\n{SHARED}\n"));
        let b2 = make_source(tmp2.path(), "beta", &format!("# Beta\n\n{SHARED}\n"));

        // Second run — should reuse cached sigs (no new sigs returned).
        let (_diags2, new_sigs) = check(&[a2, b2], &lf);
        assert!(
            new_sigs.is_empty(),
            "no new signatures should be computed when cache is warm"
        );
    }
}
