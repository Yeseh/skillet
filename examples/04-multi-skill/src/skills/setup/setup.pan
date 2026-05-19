---
name: setup
description: "Provisions the local development environment from scratch."
---

# setup

Use this skill when preparing a fresh development environment.

## Steps

1. Clone the repository with `cmd::git clone`.
2. Install dependencies.
3. Copy `.env.example` to `.env` and fill in the required values.
4. Run database migrations.
5. Start background services and verify they are healthy.

## Verification

After setup, confirm the environment is ready by running the test suite.
If any step fails, consult the project README for troubleshooting guidance.
