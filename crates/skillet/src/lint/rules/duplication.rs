//! Rule: `duplication` — warns when near-verbatim passages are shared across
//! compiled skill outputs, suggesting extraction into a fragment.

use crate::workspace::SkillSource;

use super::{Diagnostic, Severity};

/// Minimum number of consecutive sentences that must match to trigger a warning.
const MIN_SENTENCES: usize = 3;
/// Overlap ratio threshold (>80 %).
const OVERLAP_THRESHOLD: f64 = 0.80;

/// Checks all compiled `SKILL.md` outputs for near-verbatim shared passages.
///
/// Only cross-skill duplication is reported.  Skills whose `SKILL.md` is
/// missing are silently skipped (the `stale-build` rule already covers that).
pub fn check(all_sources: &[SkillSource]) -> Vec<Diagnostic> {
    // Collect (skill_name, windows) for skills that have a compiled output.
    let skill_windows: Vec<(&str, Vec<String>)> = all_sources
        .iter()
        .filter_map(|s| {
            let path = s.skill_out_dir.join("SKILL.md");
            let text = std::fs::read_to_string(&path).ok()?;
            let sents = sentences(&text);
            if sents.len() < MIN_SENTENCES {
                return None;
            }
            let wins: Vec<String> = sents.windows(MIN_SENTENCES).map(|w| w.join(" ")).collect();
            Some((s.name.as_str(), wins))
        })
        .collect();

    if skill_windows.len() < 2 {
        return vec![];
    }

    // Phase 1: find all cross-skill matching passages.
    // Each entry is (canonical_passage, skill_a, skill_b).
    let mut raw_matches: Vec<(String, &str, &str)> = Vec::new();

    for i in 0..skill_windows.len() {
        for j in (i + 1)..skill_windows.len() {
            let (name_a, wins_a) = &skill_windows[i];
            let (name_b, wins_b) = &skill_windows[j];

            for wa in wins_a {
                for wb in wins_b {
                    let ratio = overlap_ratio(wa, wb);
                    if ratio <= OVERLAP_THRESHOLD {
                        continue;
                    }
                    // Use the longer passage as canonical.
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
        return vec![];
    }

    // Phase 2: cluster near-identical passages together.
    // Each cluster holds (canonical_passage, set_of_skills).
    // We merge a new match into an existing cluster if the passage overlaps >80%
    // with the cluster's canonical passage; otherwise we start a new cluster.
    struct Cluster {
        passage: String,
        skills: std::collections::BTreeSet<String>,
    }

    let mut clusters: Vec<Cluster> = Vec::new();

    for (passage, skill_a, skill_b) in raw_matches {
        // Find an existing cluster whose canonical passage is near-identical.
        let mut merged = false;
        for cluster in &mut clusters {
            // Use a lower threshold here: adjacent 3-sentence windows sharing 2
            // sentences have ~0.67 word overlap, which is below OVERLAP_THRESHOLD.
            // 0.50 is sufficient to recognise they describe the same passage.
            if overlap_ratio(&cluster.passage, &passage) > 0.50 {
                // Same underlying passage — add skills and keep longer canonical.
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

    // Phase 3: emit one diagnostic per cluster that involves 2+ distinct skills.
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
    diags
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Splits text into normalised sentences (trims whitespace/case, drops blank).
fn sentences(text: &str) -> Vec<String> {
    // Strip YAML frontmatter (--- ... ---) and markdown headings/code fences.
    let body = strip_frontmatter(text);
    let cleaned: String = body
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("```"))
        .collect::<Vec<_>>()
        .join(" ");

    // Split on sentence-ending punctuation followed by whitespace or EOL.
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
    // Flush remainder (no trailing punctuation).
    let s = normalise(&buf);
    if !s.is_empty() {
        out.push(s);
    }
    out
}

/// Removes the leading YAML frontmatter block (`---` ... `---`) if present.
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

/// Returns the overlap ratio between two strings based on shared word tokens.
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

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
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
        let diags = check(&[a, b]);
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
        let diags = check(&[a]);
        assert!(diags.is_empty());
    }

    #[test]
    fn detects_near_verbatim_cross_skill_duplication() {
        let tmp = TempDir::new().unwrap();
        // One word differs («now» vs «today»): ratio = 12/14 ≈ 0.86 > 0.80.
        let a = make_source(
            tmp.path(),
            "alpha",
            "The quick brown fox jumps over. Second sentence is here now. Third sentence ends the passage.",
        );
        let b = make_source(
            tmp.path(),
            "beta",
            "The quick brown fox jumps over. Second sentence is here today. Third sentence ends the passage.",
        );
        let diags = check(&[a, b]);
        assert!(
            diags.iter().any(|d| d.rule == "duplication"
                && d.affected_skills
                    .as_ref()
                    .map(|v| v.contains(&"alpha".to_string()) && v.contains(&"beta".to_string()))
                    .unwrap_or(false)),
            "expected near-verbatim duplication warning, got: {diags:?}"
        );
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
        let diags = check(&[a, b]);
        assert!(diags.is_empty());
    }

    #[test]
    fn overlapping_windows_collapsed_to_single_diagnostic() {
        let tmp = TempDir::new().unwrap();
        // 5 sentences shared → produces windows [1-3], [2-4], [3-5] all matching.
        // Should collapse to ONE diagnostic, not three.
        let content = "Sentence one here. Sentence two here. Sentence three here. \
                       Sentence four here. Sentence five here.";
        let a = make_source(tmp.path(), "alpha", content);
        let b = make_source(tmp.path(), "beta", content);
        let diags = check(&[a, b]);
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
        let diags = check(&[a, b, c]);
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
}
