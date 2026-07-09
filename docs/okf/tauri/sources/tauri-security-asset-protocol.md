---
type: Source
title: "Asset protocol scope"
description: "Security model controlling which filesystem paths can be served to WebView via asset protocol."
resource: https://v2.tauri.app/security/asset-protocol/
tags: [security]
timestamp: 2026-07-09T00:00:00Z
fetched: 2026-07-09
---

# Summary

Tauri's asset protocol security model controls which filesystem paths can be served to the WebView through the custom asset protocol. Developers must enable the feature and define a scope using glob patterns, which can be expressed as an array of allowed paths or an object with allow/deny rules. The system includes special handling for Unix dot-prefixed directories and supports base directory variables like `$HOME` and `$RESOURCE`. Static configuration in `tauri.conf.json` defines permitted paths, while runtime user selections may require the persisted-scope plugin to maintain access across app restarts.
