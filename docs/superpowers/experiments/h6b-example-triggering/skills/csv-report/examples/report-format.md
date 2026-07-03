# csv-report — worked example (house format)

This is the exact output format for `report.md`. Follow it character-for-character.

## Format rules

1. **First line** is exactly: `# Report: <dataset-name>` where `<dataset-name>` is
   the CSV file name without its `.csv` extension.
2. Then a markdown table whose header row is exactly `| metric | value |` followed
   by the separator row `| --- | --- |`, then one row per metric.
3. **Every numeric value is rounded to EXACTLY 2 decimals** (e.g. `7` -> `7.00`,
   `3.5` -> `3.50`, `2.718` -> `2.72`).
4. **Final line** is exactly: `TOTAL: <sum-of-values-2dp>` — the sum of all values,
   also rounded to exactly 2 decimals.

## Concrete filled-in example

Given a CSV named `sales.csv`:

```
metric,value
revenue,1200.5
cost,800
margin,399.5
```

The correct `report.md` is exactly:

```
# Report: sales
| metric | value |
| --- | --- |
| revenue | 1200.50 |
| cost | 800.00 |
| margin | 399.50 |
TOTAL: 2400.50
```

Note: `800` became `800.00`, and `TOTAL` is the 2-decimal sum
`1200.50 + 800.00 + 399.50 = 2400.50`. No blank line before `TOTAL`.
