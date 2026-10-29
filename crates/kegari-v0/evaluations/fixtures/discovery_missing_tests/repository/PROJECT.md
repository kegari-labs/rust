# Current project contract

The limit is inclusive: values equal to the limit are accepted. Read applicable
AGENTS.md and inspect the source and ordinary tests before deriving commands.

Use an output path in the session scratch directory for structured results.
The ordinary project validation is `python3 -B scripts/validate_boundary.py --result RESULT`.
The result suite is `boundary`. A zero exit without the actual result is not a pass.
Testing work also runs `python3 -B scripts/validate_regression.py --result RESULT`
for the `regression_sensitivity` suite: the candidate tests must accept inclusive
code and reject the deliberately exclusive mutant. This is project documentation,
not Kegari configuration or a required environment manifest.
