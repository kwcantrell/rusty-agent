---
type: Source
title: "Selenium | Tauri"
description: "End-to-end testing guide for Tauri applications using Selenium WebDriver with Mocha and Chai."
resource: https://v2.tauri.app/develop/tests/webdriver/example/selenium/
tags: [testing]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Comprehensive guide for implementing end-to-end testing in Tauri applications using Selenium WebDriver. The example uses Mocha as the test framework and Chai for assertions. The documentation highlights: "With Selenium and some hooking up to a test suite, we just enabled e2e testing without modifying our Tauri application at all!"

## Setup Prerequisites

- Node.js installation with npm, yarn, or pnpm
- Completion of WebDriver manual setup instructions
- A Tauri application ready for testing

## Project Structure

Create a dedicated `e2e-tests` directory for test files:

```bash
mkdir -p e2e-tests
```

## Dependencies

The example uses three primary packages:

```json
{
  "devDependencies": {
    "mocha": "^11.7.1",
    "chai": "^5.2.1",
    "selenium-webdriver": "^4.34.0"
  }
}
```

### Dependency Roles

- **Mocha** (v11.7.1): Testing framework that organizes and executes test suites
- **Chai** (v5.2.1): Assertion library for fluent assertion syntax
- **selenium-webdriver** (v4.34.0): Node.js client implementation of the WebDriver protocol

## Test Implementation Example

The test file (`test/test.js`) structure includes:

### Before Hooks
- Build the Tauri application
- Start the `tauri-driver` process on `127.0.0.1:4444`
- Create a Selenium WebDriver session targeting the built application

### Test Cases

Three test cases validating application behavior:

1. **Greeting Text Validation**: Verifies the application displays the expected greeting message
2. **Punctuation Check**: Confirms proper punctuation in displayed text
3. **Background Color Property**: Validates that elements use readable background colors

### After Hooks
- Properly close the WebDriver session
- Terminate the `tauri-driver` process

## Test Execution

Run tests via npm or yarn:

```bash
npm test
# or
yarn test
```

The example demonstrates three passing tests completing in approximately 588 milliseconds.

## Key Code Pattern

```javascript
// Before hook
before(async function() {
  // Build Tauri app
  // Start tauri-driver
  // Create WebDriver session
});

// Test example
it('should display greeting', async function() {
  const greeting = await driver.findElement(By.id('greeting')).getText();
  expect(greeting).to.equal('Welcome!');
});

// After hook
after(async function() {
  await driver.quit();
  // Terminate tauri-driver
});
```

## Advantages

The Selenium approach requires no modification to the Tauri application itself, enabling comprehensive automated testing through the WebDriver protocol. Tests interact with the application through standard W3C WebDriver semantics, leveraging mature ecosystem tooling and best practices from web automation.
