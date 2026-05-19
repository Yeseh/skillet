---
name: deploy
description: "Guides deployment of the application to the configured environment."
---

# deploy

Use this skill when deploying `var::project_name` to the `env::DEPLOY_ENV` environment.

## Pre-flight Checks

Before deploying, confirm:

- Project: `var::project_name`
- Target environment: `env::DEPLOY_ENV`
- CI is passing: `env::CI`

## Steps

1. Tag the release commit with `cmd::git tag`.
2. Build the container image.
3. Push the image and trigger the `env::DEPLOY_ENV` deployment pipeline.
4. Monitor the rollout and verify all health checks pass.
5. Announce the deployment in the team channel.
