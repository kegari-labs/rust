---
schema_version: 1
hooks:
  timeout_ms: 60000
checks:
  local:
    required-suite:
      argv: ["./scripts/validate-local", "--result", ".kegari-output/local.json"]
      result_artifact: .kegari-output/local.json
  integrated:
    required-e2e:
      argv: ["./scripts/validate-integrated", "--result", ".kegari-output/integrated.json"]
      result_artifact: .kegari-output/integrated.json
---
# Repository work for {{ workspace.name }}

Phase: {{ phase }}. Attempt: {{ attempt }}.
Execution profile: {{ profile | json }}.
Operation: {{ operation | json }}.

## Requested outcome and context

Parent intent: {{ parent | json }}
Assigned work: {{ issue | json }}
Required children: {{ children | json }}
Authorized boundary: {{ scope | json }}
Dependency results: {{ dependencies | json }}
Current corrections: {{ corrections | json }}
Actual prior evidence: {{ evidence | json }}

Follow the applicable repository instructions and use this isolated execution
cwd. Inspect the concise repository entry point and referenced architecture or
conventions needed for the task. If those required resources or the configured
validation scripts are absent, report the missing prerequisite. These example
script names must be replaced or supplied by the target repository owner before
configuration can pass preflight; this starter does not establish access/readiness.

{% if phase == "review" %}
Review the exact requested change and generation. Report concrete findings, or
an evidenced clean outcome. Do not implement speculative changes.
{% elsif phase == "coordination" %}
Identify affected existing children and propose actionable scoped routing.
Do not implement at parent level.
{% else %}
Fulfill the assigned responsibility within the declared boundary. Run required
checks, preserve exact revision and artifact references, and report remaining
blockers or an evidence-supported completion/no-change proposal.
{% endif %}
