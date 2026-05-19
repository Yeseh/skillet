---
name: teardown
description: "Tears down the local development environment created by the setup skill."
---

# teardown

Use this skill when cleaning up after development work.
Run `skill::setup` first if the environment is not yet provisioned.

## Steps

1. Stop all running services gracefully.
2. Drop the local database (confirm with the user before proceeding).
3. Remove generated artefacts and build caches.
4. Optionally delete the `.env` file if it contains sensitive credentials.

## Notes

This skill is the inverse of `skill::setup`.
All data in the local database will be lost — ensure nothing important is unsaved.
