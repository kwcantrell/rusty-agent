# Tauri OKF bundle — source allowlist (Wave 0, owner-approved contract)

Curated 2026-07-09 against docs/superpowers/specs/2026-07-09-tauri-okf-bundle-design.md
from a 125-page nav-tree scout + 26-candidate ecosystem scout.
Slugs are unique; depth: deep = full condensation, must be cited by ≥1 concept;
survey = short abstract, may end up uncited.
**Tauri stable at curation: 2.11.4** (npm @tauri-apps/cli; docs brand as "v2").
Total: 45 rows (10 testing, 12 security, 6 ipc-architecture, 6 core,
5 distribution, 3 performance, 3 mobile). Deep tags: testing 9, security 8 —
the owner's emphasis is carried by the corpus.

## testing (10 — 9 deep)

| slug | url | area | depth |
|---|---|---|---|
| tauri-tests-overview | https://v2.tauri.app/develop/tests/ | testing | deep |
| tauri-tests-mocking | https://v2.tauri.app/develop/tests/mocking/ | testing | deep |
| tauri-js-mocks-reference | https://v2.tauri.app/reference/javascript/api/namespacemocks/ | testing | deep |
| tauri-webdriver-overview | https://v2.tauri.app/develop/tests/webdriver/ | testing | deep |
| tauri-webdriver-manual-setup | https://v2.tauri.app/develop/tests/webdriver/manual-setup/ | testing | deep |
| tauri-webdriver-ci | https://v2.tauri.app/develop/tests/webdriver/ci/ | testing | deep |
| tauri-webdriver-selenium | https://v2.tauri.app/develop/tests/webdriver/example/selenium/ | testing | deep |
| tauri-webdriver-webdriverio | https://v2.tauri.app/develop/tests/webdriver/example/webdriverio/ | testing | deep |
| wdio-tauri-service | https://webdriver.io/docs/desktop-testing/tauri/platform-support/ | testing | deep |
| wkwebview-driver-macos | https://danielraffel.me/2026/02/14/i-built-a-webdriver-for-wkwebview-tauri-apps-on-macos/ | testing | survey |

## security (12 — 8 deep)

| slug | url | area | depth |
|---|---|---|---|
| tauri-security-overview | https://v2.tauri.app/security/ | security | deep |
| tauri-security-permissions | https://v2.tauri.app/security/permissions/ | security | deep |
| tauri-security-scope | https://v2.tauri.app/security/scope/ | security | deep |
| tauri-security-capabilities | https://v2.tauri.app/security/capabilities/ | security | deep |
| tauri-security-csp | https://v2.tauri.app/security/csp/ | security | deep |
| tauri-security-lifecycle | https://v2.tauri.app/security/lifecycle/ | security | survey |
| tauri-ipc-isolation | https://v2.tauri.app/concept/inter-process-communication/isolation/ | security | deep |
| tauri-learn-plugin-permissions | https://v2.tauri.app/learn/security/using-plugin-permissions/ | security | deep |
| tauri-learn-capabilities-multiwindow | https://v2.tauri.app/learn/security/capabilities-for-windows-and-platforms/ | security | deep |
| tauri-security-asset-protocol | https://v2.tauri.app/security/asset-protocol/ | security | survey |
| ghsa-local-url-origin-confusion | https://github.com/tauri-apps/tauri/security/advisories/GHSA-7gmj-67g7-phm9 | security | survey |
| bishopfox-beyond-electron | https://bishopfox.com/blog/beyond-electron-attacking-alternative-desktop-application-frameworks | security | survey |

## ipc-architecture (6 — 4 deep)

| slug | url | area | depth |
|---|---|---|---|
| tauri-ipc-overview | https://v2.tauri.app/concept/inter-process-communication/ | ipc-architecture | deep |
| tauri-calling-rust | https://v2.tauri.app/develop/calling-rust/ | ipc-architecture | deep |
| tauri-calling-frontend | https://v2.tauri.app/develop/calling-frontend/ | ipc-architecture | deep |
| tauri-state-management | https://v2.tauri.app/develop/state-management/ | ipc-architecture | deep |
| tauri-discussion-ipc-improvements | https://github.com/orgs/tauri-apps/discussions/5690 | ipc-architecture | survey |
| tauri-discussion-channel-blocking | https://github.com/tauri-apps/tauri/discussions/11589 | ipc-architecture | survey |

## core (6 — 4 deep)

| slug | url | area | depth |
|---|---|---|---|
| tauri-architecture | https://v2.tauri.app/concept/architecture/ | core | deep |
| tauri-process-model | https://v2.tauri.app/concept/process-model/ | core | deep |
| tauri-plugins-develop | https://v2.tauri.app/develop/plugins/ | core | deep |
| tauri-migrate-from-v1 | https://v2.tauri.app/start/migrate/from-tauri-1/ | core | deep |
| tauri-plugin-catalog | https://v2.tauri.app/plugin/ | core | survey |
| tauri-window-customization | https://v2.tauri.app/learn/window-customization/ | core | survey |

## distribution (5 — 5 deep)

| slug | url | area | depth |
|---|---|---|---|
| tauri-distribute-overview | https://v2.tauri.app/distribute/ | distribution | deep |
| tauri-sign-macos | https://v2.tauri.app/distribute/sign/macos/ | distribution | deep |
| tauri-sign-windows | https://v2.tauri.app/distribute/sign/windows/ | distribution | deep |
| tauri-plugin-updater | https://v2.tauri.app/plugin/updater/ | distribution | deep |
| tauri-pipelines-github | https://v2.tauri.app/distribute/pipelines/github/ | distribution | deep |

## performance (3 — 1 deep)

| slug | url | area | depth |
|---|---|---|---|
| tauri-app-size | https://v2.tauri.app/concept/size/ | performance | deep |
| tauri-benchmark-results | https://github.com/tauri-apps/benchmark_results | performance | survey |
| tauri-issue-memory-benchmarks | https://github.com/tauri-apps/tauri/issues/5889 | performance | survey |

## mobile (3 — 0 deep, survey per spec)

| slug | url | area | depth |
|---|---|---|---|
| tauri-mobile-plugin-dev | https://v2.tauri.app/develop/plugins/develop-mobile/ | mobile | survey |
| tauri-sign-android | https://v2.tauri.app/distribute/sign/android/ | mobile | survey |
| tauri-prerequisites | https://v2.tauri.app/start/prerequisites/ | mobile | survey |

## Curation notes

- Provenance: all rows are first-party (v2.tauri.app / github.com/tauri-apps) except
  `wdio-tauri-service` (WebdriverIO project docs — the docs-recommended E2E path),
  `wkwebview-driver-macos` (maintainer-adjacent deep-dive on the acknowledged macOS
  WebDriver gap), and `bishopfox-beyond-electron` (established security firm's
  offensive analysis). GitHub discussions are extraction-scoped to maintainer comments.
- Dropped from candidates: framework-integration pages (Leptos/Next/Nuxt/etc — not
  practice-bearing), per-plugin catalog pages except updater (capability filler; the
  catalog index covers the ecosystem), debug-IDE setup pages, About/governance,
  Linux-packaging long tail (AppImage/AUR/deb/RPM/snap — distribute overview covers
  the map), CrabNebula commercial pages, npm package page (redundant with wdio docs),
  org discussion #3768 on Rust E2E (superseded by official WebDriver docs).
