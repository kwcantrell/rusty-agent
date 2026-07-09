# Sources

Snapshotted source material (evidence layer). Provenance: `resource` = live URL, `fetched` = snapshot date.

## testing

* [mocks](/sources/tauri-js-mocks-reference.md) - Reference for the @tauri-apps/api/mocks namespace — mockIPC, mockWindows, clearMocks
* [Mock Tauri APIs](/sources/tauri-tests-mocking.md) - Mock Tauri APIs
* [Tests](/sources/tauri-tests-overview.md) - Overview of Tauri unit/integration testing under a mock runtime, plus WebDriver E2E
* [Continuous Integration | Tauri](/sources/tauri-webdriver-ci.md) - GitHub Actions workflow for executing WebDriver tests with tauri-driver on Linux and Windows CI systems.
* [Manual setup](/sources/tauri-webdriver-manual-setup.md) - Manual tauri-driver WebDriver setup for Windows and Linux
* [WebDriver](/sources/tauri-webdriver-overview.md) - Overview of WebDriver E2E testing for Tauri via @wdio/tauri-service and tauri-driver
* [Selenium | Tauri](/sources/tauri-webdriver-selenium.md) - End-to-end testing guide for Tauri applications using Selenium WebDriver with Mocha and Chai.
* [WebdriverIO | Tauri](/sources/tauri-webdriver-webdriverio.md) - End-to-end testing setup for Tauri applications using WebdriverIO and tauri-driver integration.
* [Platform Support | WebdriverIO](/sources/wdio-tauri-service.md) - Platform-specific WebDriver setup and support matrix for Tauri testing on Windows, Linux, and macOS.
* [I Built a WebDriver for WKWebView Tauri Apps on macOS](/sources/wkwebview-driver-macos.md) - Open-source W3C WebDriver implementation for end-to-end testing of Tauri applications on macOS.

## security

* [Beyond Electron: Attacking Alternative Desktop Application Frameworks](/sources/bishopfox-beyond-electron.md) - Bishop Fox analysis of XSS and misconfiguration vulnerabilities in Tauri-based desktop applications.
* [Origin Confusion Allows Remote Pages to Invoke Local-Only IPC Commands](/sources/ghsa-local-url-origin-confusion.md) - CVE-2026-42184: Tauri is_local_url() validation flaw on Windows/Android allows remote domains to spoof local origins.
* [Isolation Pattern | Tauri](/sources/tauri-ipc-isolation.md) - IPC isolation pattern intercepts and validates frontend-to-Tauri API messages via sandboxed iframe.
* [Capabilities for Different Windows and Platforms](/sources/tauri-learn-capabilities-multiwindow.md) - Capability-based access control for assigning different permissions to windows and platforms.
* [Using Plugin Permissions](/sources/tauri-learn-plugin-permissions.md) - How to enable, disable, and customize plugin permissions in Tauri applications.
* [Asset protocol scope](/sources/tauri-security-asset-protocol.md) - Security model controlling which filesystem paths can be served to WebView via asset protocol.
* [Capabilities | Tauri](/sources/tauri-security-capabilities.md) - Tauri capabilities system for granularly controlling permissions granted to windows and webviews.
* [Content Security Policy (CSP) | Tauri](/sources/tauri-security-csp.md) - Content Security Policy implementation in Tauri to mitigate cross-site-scripting and related web vulnerabilities.
* [Application Lifecycle Threats](/sources/tauri-security-lifecycle.md) - Security risks across the entire Tauri application lifecycle from dependencies through runtime.
* [Security | Tauri](/sources/tauri-security-overview.md) - Overview of Tauri's security architecture, trust boundaries, access control mechanisms, and vulnerability disclosure process.
* [Permissions | Tauri](/sources/tauri-security-permissions.md) - Tauri permissions system for controlling explicit privileges of commands and frontend access to system resources.
* [Command Scopes | Tauri](/sources/tauri-security-scope.md) - Granular command scope mechanism for controlling permitted and restricted behaviors of Tauri commands.

## ipc-architecture

* [Calling the Frontend from Rust](/sources/tauri-calling-frontend.md) - Tauri API reference for Rust code to communicate with frontend via events, channels, and JavaScript evaluation.
* [Calling Rust from the Frontend](/sources/tauri-calling-rust.md) - Tauri API reference for invoking Rust functions from JavaScript with examples of commands, events, and streaming.
* [How can I make Channel::send non-blocking? · tauri-apps · Discussion #11589 · GitHub](/sources/tauri-discussion-channel-blocking.md) - User-reported Channel::send blocking behavior (30-50ms) when transmitting large video frame data.
* [IPC Improvements · tauri-apps · Discussion #5690 · GitHub](/sources/tauri-discussion-ipc-improvements.md) - Maintainer discussion on IPC performance bottlenecks, serialization constraints, and platform-specific implementation strategies.
* [Inter-Process Communication | Tauri](/sources/tauri-ipc-overview.md) - Comprehensive overview of Tauri's asynchronous message-passing IPC model and its two core primitives.
* [State Management | Tauri](/sources/tauri-state-management.md) - Built-in state management system through Manager API with interior mutability patterns for shared state across commands and threads.

## core

* [Tauri Architecture | Tauri](/sources/tauri-architecture.md) - Polyglot composable toolkit combining Rust with HTML rendered in Webview through message passing between frontend and backend.
* [Upgrade from Tauri 1.0 | Tauri](/sources/tauri-migrate-from-v1.md) - Comprehensive migration guide from Tauri 1.0 to 2.0 covering configuration, API restructuring, and breaking changes.
* [Features & Recipes | Tauri](/sources/tauri-plugin-catalog.md) - Catalog of official and community Tauri plugins covering file system, notifications, HTTP, biometric authentication, and platform integrations.
* [Plugin Development | Tauri](/sources/tauri-plugins-develop.md) - Tauri plugins extend core functionality via Rust, Kotlin, or Swift code exposed to the webview.
* [Process Model | Tauri](/sources/tauri-process-model.md) - Multi-process architecture separating Core and WebView processes for resilience, performance, and security through privilege isolation.
* [Window Customization | Tauri](/sources/tauri-window-customization.md) - Tauri provides configuration, JavaScript, and Rust APIs for customizing window appearance and behavior.

## distribution

* [Distribute | Tauri](/sources/tauri-distribute-overview.md) - Comprehensive tooling for releasing Tauri applications to app stores or as platform-specific installers.
* [GitHub Actions Pipeline for Tauri](/sources/tauri-pipelines-github.md) - CI/CD setup using tauri-action for multi-platform building, signing, and automated release workflows.
* [Tauri Updater Plugin](/sources/tauri-plugin-updater.md) - Automatic application updates via dynamic server or static JSON with mandatory cryptographic signature validation.
* [macOS Code Signing](/sources/tauri-sign-macos.md) - Apple Developer setup, certificate creation, and CI/CD pipeline configuration for macOS application signing.
* [Windows Code Signing](/sources/tauri-sign-windows.md) - Code signing via OV certificates, Azure Key Vault, and Azure Artifact Signing for Microsoft Store distribution and SmartScreen bypass.

## performance

* [App Size Optimization in Tauri](/sources/tauri-app-size.md) - Binary size reduction through Cargo profile configuration, link-time optimization, and unused command removal.
* [tauri-apps/benchmark_results](/sources/tauri-benchmark-results.md) - Performance benchmark repository comparing Tauri, Wry, and Electron across execution time, binary size, memory usage, thread count, and syscall metrics.
* [Memory benchmark might be incorrect: Tauri might consume more RAM than Electron](/sources/tauri-issue-memory-benchmarks.md) - GitHub issue #5889 challenging Tauri's memory benchmarks and reporting WebKit memory consumption discrepancies with Electron.

## mobile

* [Mobile Plugin Development | Tauri](/sources/tauri-mobile-plugin-dev.md) - Guide for developing native mobile plugins for Tauri applications using Kotlin/Java for Android and Swift for iOS.
* [Prerequisites | Tauri](/sources/tauri-prerequisites.md) - Foundational setup requirements for building Tauri applications including system dependencies, Rust, Node.js, and optional mobile configuration.
* [Android Code Signing | Tauri](/sources/tauri-sign-android.md) - Guide to digitally signing Android App Bundles and APKs for Play Store distribution using keytool and Gradle configuration.
