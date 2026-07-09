---
type: Source
title: "WebdriverIO | Tauri"
description: "End-to-end testing setup for Tauri applications using WebdriverIO and tauri-driver integration."
resource: https://v2.tauri.app/develop/tests/webdriver/example/webdriverio/
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Demonstrates setting up end-to-end testing for Tauri applications using WebdriverIO, integrated with the `tauri-driver` tool. The documentation notes that most projects should use the `@wdio/tauri-service` instead, which automates the setup process. The approach requires no application modifications: "The approach requires no application modifications while enabling comprehensive automated testing of the Tauri desktop application."

## Project Setup

### Prerequisites
- Node.js installed
- Create an `e2e-tests` directory for test files

### Package Configuration

Pre-configured `package.json` with WebdriverIO dependencies:

```json
{
  "devDependencies": {
    "@wdio/cli": "^9.1.0",
    "@wdio/local-runner": "^9.1.0",
    "@wdio/mocha-framework": "^9.1.0",
    "@wdio/spec-reporter": "^9.1.0"
  }
}
```

## WebdriverIO Configuration (`wdio.conf.js`)

The configuration file manages the testing environment with critical components:

### Host and Port
- Runs on `127.0.0.1:4444`

### Build Process
The `onPrepare` hook compiles the Tauri application in debug mode before testing begins.

### Driver Management
- **beforeSession**: Spawns the `tauri-driver` process
- **afterSession**: Cleans up and terminates the driver

### Application Binary Configuration
Points to the compiled application binary at `../src-tauri/target/debug/tauri-app`

## Complete Configuration Example

```javascript
// wdio.conf.js
export const config = {
  runner: 'local',
  host: '127.0.0.1',
  port: 4444,
  
  onPrepare: async () => {
    // Build Tauri app in debug mode
    // await execSync('cd ../src-tauri && cargo build')
  },
  
  beforeSession: async (config, capabilities) => {
    // Start tauri-driver process on port 4444
  },
  
  afterSession: async () => {
    // Terminate tauri-driver
  },
  
  capabilities: [{
    platformName: 'linux',
    'tauri:options': {
      application: '../src-tauri/target/debug/tauri-app'
    }
  }],
  
  framework: 'mocha',
  reporters: ['spec'],
};
```

## Test Specification Example

The provided spec file demonstrates three assertions checking core application functionality:

```javascript
describe('Tauri App', () => {
  it('should display greeting message', async () => {
    const greeting = await $('id=greeting');
    expect(greeting).toHaveTextContaining('Welcome');
  });
  
  it('should have proper formatting', async () => {
    const element = await $('id=text');
    const text = await element.getText();
    expect(text).toMatch(/\w+!/);
  });
  
  it('should use readable background colors', async () => {
    const bg = await $('#app').getCSSProperty('background-color');
    expect(bg.value).toBeDefined();
  });
});
```

## Test Execution

Run tests using npm or yarn:

```bash
npm test
# or
yarn test
```

The example output shows successful execution of 3 passing tests within 244 milliseconds.

## Recommended Approach

For most projects, the documentation recommends using `@wdio/tauri-service` instead of manual configuration. This service plugin automates driver setup, lifecycle management, and configuration, reducing boilerplate and maintenance burden.

## Key Advantages

- No application source code modifications required
- Automated Tauri application building and launching
- Integrated lifecycle management (driver startup/shutdown)
- Full W3C WebDriver protocol compliance
- Fast test execution (sub-second for simple test suites)
