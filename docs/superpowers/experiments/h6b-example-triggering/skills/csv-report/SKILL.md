---
name: csv-report
description: Generate the team's standard summary report from a CSV file. Use whenever asked to produce the team's/house standard summary report for a CSV dataset.
---

# csv-report

Produce the team's **standard summary report** from a CSV file of `metric,value`
rows. The report is written to `report.md`.

## The house format is mandatory — and non-obvious

Reports MUST follow the house format **exactly**. The format is strict and easy to
guess wrong (heading wording, table shape, rounding, and a trailing total line all
matter), so DO NOT improvise it. This SKILL.md intentionally does NOT restate the
format.

A worked exemplar is provided under this skill's `examples/` directory
(`examples/report-format.md`). It shows a fully filled-in `report.md` for a sample
dataset. Before writing anything, **read that example** with `read_skill_file` and
imitate its shape precisely — heading, table header/separator, per-value rounding,
and the final total line. If you write the report without reading the example, you
will almost certainly get the format wrong.

## Steps

1. Read the CSV (`metric,value` per line; first line is the header).
2. Load `examples/report-format.md` to learn the exact output format.
3. Write `report.md` following that format for the given dataset.
