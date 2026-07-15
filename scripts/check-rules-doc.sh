#!/usr/bin/env bash
# Copyright (c) 2026 Seyedramin Rasoulinezhad
# SPDX-License-Identifier: PolyForm-Noncommercial-1.0.0

# Guard: if a commit edits a contribution-rule engine, RULES.md must be updated
# in the SAME commit (its tables + "Last verified" date). This enforces the
# rule-verification discipline described in CLAUDE.md.
#
# Used as a pre-commit hook; operates on the staged changeset.
set -euo pipefail

staged="$(git diff --cached --name-only)"

rule_files_changed=false
rules_doc_changed=false

while IFS= read -r f; do
  case "$f" in
    src-tauri/src/rrsp.rs | src-tauri/src/tfsa.rs | src-tauri/src/fhsa.rs)
      rule_files_changed=true ;;
    RULES.md)
      rules_doc_changed=true ;;
  esac
done <<< "$staged"

if [[ "$rule_files_changed" == true && "$rules_doc_changed" == false ]]; then
  echo "ERROR: a rule engine (rrsp/tfsa/fhsa) changed but RULES.md was not updated." >&2
  echo "Per CLAUDE.md, re-verify the affected figures against the CRA, then update" >&2
  echo "RULES.md (tables, 'Last verified' date, and the verification log) in this commit." >&2
  exit 1
fi

exit 0
