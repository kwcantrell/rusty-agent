---
type: Source
title: "Platform Support | WebdriverIO"
description: "Platform-specific WebDriver setup and support matrix for Tauri testing on Windows, Linux, and macOS."
resource: https://webdriver.io/docs/desktop-testing/tauri/platform-support/
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Comprehensive guidance on platform-specific requirements and WebDriver setup for Tauri application testing across Windows, Linux, and macOS environments.

## Platform Support Matrix

| Platform | Status | WebDriver | Setup |
|----------|--------|-----------|-------|
| Windows | ✅ Supported | Microsoft Edge WebDriver | Auto-managed |
| Linux | ✅ Supported | WebKitWebDriver | Manual install |
| macOS | ✅ Supported | Built-in | No external driver |

## Windows Testing

### WebDriver: Microsoft Edge WebDriver (msedgedriver.exe)

The `@wdio/tauri-service` can automatically detect WebView2 versions, download matching drivers, and handle version conflicts.

**Automatic Management:**
```javascript
config.services = [['@wdio/tauri-service', {
  autoDownloadEdgeDriver: true,
}]];
```

**Manual Setup:**
If auto-management encounters issues, users must match driver versions with their WebView2 runtime. Version mismatches are the most common cause of Windows test failures.

## Linux Testing

### WebDriver: WebKitWebDriver

**Installation by Distribution:**
- **Debian/Ubuntu**: Fully supported via package managers
- **Fedora**: Fully supported
- **Arch Linux**: Fully supported
- **Alpine Linux**: Can only be used as a runtime container, not for building Tauri apps
- **CentOS Stream**: Lacks adequate package support
- **openSUSE**: Lacks adequate package support

**Headless Testing:**
For CI/CD environments without graphical displays, Xvfb (X Virtual Framebuffer) provides display emulation:

```bash
xvfb-run -a yarn test
```

The documentation notes: "For headless testing in CI/CD, Xvfb (X Virtual Framebuffer) provides display emulation without requiring a graphical interface."

## macOS Testing

### WebDriver: Built-in Support

The embedded provider is recommended, requiring installation of `tauri-plugin-wdio-webdriver` within the Tauri application itself.

**Key Features:**
- No external drivers needed
- Service auto-detects embedded plugin configuration automatically
- Simplest setup path of all three platforms

**Alternative: CrabNebula**
CrabNebula offers a cross-platform alternative but requires:
- Paid subscription
- API key configuration
- Additional setup steps

## Driver Provider Options

Three driver providers are available across platforms:

### 1. Official (Default)
Platform-specific drivers with auto-management capabilities:
- Windows: Microsoft Edge WebDriver with auto-download
- Linux: WebKitWebDriver (manual install)
- macOS: Built-in support

### 2. Embedded
Plugin-based approach with no external driver dependency:
- Requires Tauri plugin installation in app
- Best for macOS
- Simplifies distribution and CI setup

### 3. CrabNebula
Cross-platform commercial option:
- Centralized driver management
- Requires subscription and API key
- Handles platform-specific complexity centrally

## Platform-Specific Configuration Example

```javascript
// wdio.conf.ts
export const config = {
  onPrepare: async (config, specs) => {
    const platform = process.platform;
    
    if (platform === 'win32') {
      config.services = [['@wdio/tauri-service', {
        autoDownloadEdgeDriver: true,
      }]];
    } else if (platform === 'linux') {
      config.services = [['@wdio/tauri-service', {}]];
      // Ensure WebKitWebDriver is installed
    } else if (platform === 'darwin') {
      config.services = [['@wdio/tauri-service', {}]];
      // Uses built-in macOS support
    }
  },
};
```

## Common Issues

**Windows:** Driver version mismatch with WebView2 runtime — auto-download mitigates this.

**Linux:** WebKitWebDriver not installed for distribution — install manually or use container.

**macOS:** Plugin not installed in app — service auto-detection will fail; verify plugin presence.

## CI/CD Recommendations

- **Windows**: Use `autoDownloadEdgeDriver: true` for automatic version management
- **Linux**: Use `xvfb-run` in headless CI environments
- **macOS**: Prefer embedded plugin approach; GHA runners include xcode-select support
