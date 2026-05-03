# Changelog


## v0.22.6 (2026-05-03)

### Features

- vpn manipulation via the api

### Refactoring

- don't block ui on clade check

### Documentation

- update CHANGELOG.md and README.md for v0.22.5 [skip ci] (#327)

### Maintenance

- chore: version bump
- chore: rand bump
- chore: pnpm bump
- ci(deps): bump the github-actions group with 3 updates (#330)
- chore: update flake.nix for v0.22.5 [skip ci] (#328)

### Other

- deps(rust)(deps): bump the rust-dependencies group (#331)


## v0.22.5 (2026-04-29)

### Bug Fixes

- declare libxdo as runtime dependency

### Maintenance

- chore: version bump
- chore: copy
- chore: update flake.nix for v0.22.4 [skip ci] (#324)


## v0.22.4 (2026-04-28)

### Maintenance

- chore: version bump
- chore: i18n
- chore: update flake.nix for v0.22.3 [skip ci] (#321)


## v0.22.3 (2026-04-27)

### Bug Fixes

- correct browser port mapping

### Maintenance

- chore: version bump
- chore: update flake.nix for v0.22.2 [skip ci] (#315)


## v0.22.2 (2026-04-27)

### Refactoring

- cookie management

### Maintenance

- chore: version bump
- chore: update flake.nix for v0.22.1 [skip ci] (#313)


## v0.22.1 (2026-04-27)

### Bug Fixes

- link proper wayfern tos

### Refactoring

- vpn refresh and remove openvpn support

### Documentation

- update CHANGELOG.md and README.md for v0.22.0 [skip ci] (#306)

### Maintenance

- chore: version bump
- chore: linting
- chore: audit
- chore: update flake.nix for v0.22.0 [skip ci] (#307)

### Other

- deps(rust)(deps): bump the rust-dependencies group across 1 directory with 34 updates (#305)


## v0.22.0 (2026-04-25)

### Refactoring

- auth and wayfern
- cdp gates cleanup

### Maintenance

- chore: tests
- chore:cargo audit
- chore: version bump
- chore: ignore .claude
- chore: update flake.nix for v0.21.2 [skip ci] (#298)


## v0.21.2 (2026-04-21)

### Bug Fixes

- properly handle headless mode

### Maintenance

- chore: version bump
- chore: update flake.nix for v0.21.1 [skip ci] (#295)


## v0.21.1 (2026-04-19)

### Features

- shadowsocks

### Refactoring

- better cleanup
- proxy cleanup

### Maintenance

- chore: version bump
- chore: linting
- ci(deps): bump the github-actions group with 3 updates
- chore: update flake.nix for v0.21.0 [skip ci] (#289)


## v0.21.0 (2026-04-16)

### Features

- shadowsocks

### Bug Fixes

- vpn config discovery

### Refactoring

- cleanup
- stricter proxy cleanup
- wayfern launch
- better error handling
- self-updates
- x64 performance

### Maintenance

- chore: version bump
- chore: proper formatting
- chore: remove pre-installed aws cli
- chore: update flake.nix for v0.20.4 [skip ci] (#283)

### Other

- deps(rust)(deps): bump rand from 0.10.0 to 0.10.1 in /src-tauri (#285)
- style: button should not become bigger on hover
- style: scrollbars


## v0.20.4 (2026-04-11)

### Refactoring

- vpn
- save port

### Maintenance

- chore: version bump
- chore: linting
- chore: overwrite aws cli
- ci(deps): bump the github-actions group with 3 updates
- chore: update flake.nix for v0.20.3 [skip ci] (#278)

### Other

- style: copy
- deps(rust)(deps): bump the rust-dependencies group
- deps(deps): bump next from 16.2.2 to 16.2.3


## v0.20.3 (2026-04-10)

### Refactoring

- debug wayfern launch

### Maintenance

- chore: version bump
- chore: serialize changelog and flake jobs
- chore: update flake.nix for v0.20.2 [skip ci] (#273)


## v0.20.2 (2026-04-08)

### Maintenance

- chore: version bump
- chore: aws integrity checks
- chore: inject NEXT_PUBLIC_TURNSTILE everywhere
- chore: update flake.nix for v0.20.1 [skip ci] (#272)


## v0.20.1 (2026-04-08)

### Maintenance

- chore: version bump
- chore: normalize r2 endpoint
- chore: pull turnstile public key in frontend at build time
- chore: update flake.nix for v0.20.0 [skip ci] (#270)


## v0.20.0 (2026-04-08)

### Bug Fixes

- cookie copying for wayfern

### Refactoring

- cleanup
- dynamic proxy

### Documentation

- update CHANGELOG.md and README.md for v0.19.0 [skip ci] (#261)

### Maintenance

- chore: version bump
- chore: linting
- chore: linting
- chore: linting
- chore: update flake.nix for v0.19.0 [skip ci] (#262)

### Other

- deps(rust)(deps): bump the rust-dependencies group
- deps(deps): bump the frontend-dependencies group with 19 updates


## v0.19.0 (2026-04-04)

### Features

- captcha on email input
- dns block lists
- portable build

### Bug Fixes

- follow latest MCP spec
- wayfern initial connection on macos doesn't timeout

### Refactoring

- linux auto updates
- more robust vpn handling
- don't allow portable build to be set as the default browser
- show app version in settings

### Documentation

- remove codacy badge
- agents
- contrib-readme-action has updated readme
- update CHANGELOG.md and README.md for v0.18.1 [skip ci]
- cleanup

### Maintenance

- test: simplify
- chore: preserve cargo
- chore: version bump
- chore: linting
- chore: update dependencies
- chore: repo publish workflow
- chore: copy and backlink
- test: serialize
- chore: copy correct file
- chore: linting
- chore: do not provide possible cause
- chore: linting
- chore: linting
- chore: linting
- chore: linting
- ci(deps): bump the github-actions group with 8 updates
- chore: commit doc changes directly and pretty discord notifications
- chore: update flake.nix for v0.18.1 [skip ci]
- chore: fix linting and formatting

### Other

- deps(deps): bump the frontend-dependencies group with 35 updates
- deps(rust)(deps): bump the rust-dependencies group

## v0.18.1 (2026-03-24)

### Refactoring

- run docker workflow on release

### Documentation

- agents.md

### Maintenance

- chore: version bump
- chore: require ai disclosure
- chore: redeploy web on new release
- chore: fix e2e in pr requests
- chore: issues get stale after 30 days
- chore: better issue validation
- chore: update flake.nix for v0.18.0 [skip ci] (#247)

