# Capabilities

* [Process model](/capabilities/process-model.md) - the multi-process split between a privileged Rust Core and OS WebView renderers, and why least privilege shapes it
* [IPC surface](/capabilities/ipc-surface.md) - commands, events, and channels; their contracts, the JSON serialization ceiling, and the Isolation hardening path
* [Windowing](/capabilities/windowing.md) - window creation and customization (decorations, custom titlebars, drag, constraints) and its per-window security tie-in
* [Plugin system](/capabilities/plugin-system.md) - plugin architecture, lifecycle hooks, permission integration, and the official/community ecosystem
* [System-webview model](/capabilities/webview.md) - rendering via OS webviews (WebView2/WKWebView/WebKitGTK): the size/security upside and the per-platform, unpinned-engine downside
* [Mobile support](/capabilities/mobile.md) - survey of Android/iOS reach: shared core, the native plugin split, project layout, and the desktop/mobile capability line
