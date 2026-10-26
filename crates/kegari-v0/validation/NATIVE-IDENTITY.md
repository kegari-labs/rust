# NQ-01A: native host and owned-process identity

The user explicitly approved one bounded read-only native test outside the
sandbox. It ran once and **passed**: one selected test, zero failures, nine
filtered tests, empty stderr and no files created in its disposable working
directory. The wrapper completed in 0.384 seconds without reaching its
20-second timeout. No setup, Keychain, installation, service, provider or
existing-process signalling operation was performed.

The selected test was
`native_host_and_birth_identity_are_observed_without_environment_trust` from the
preserved alpha.3 release-profile platform test executable. It discovered the
actual OS account/home, host ID and boot UUID; verified its own process identity;
rejected an altered process birth identity; and observed positive available
storage. It spawned no child. Raw host/account identifiers were not logged.

Exact executable:
`target/native-qualification/nq-01-91160a62001a/platform-test`.
SHA-256:
`91160a62001a6439c43cc4ffdca16879f5e69a23dcb9826738ee46bd6cb182fa`.
Arguments:

```text
--ignored --exact native_host_and_birth_identity_are_observed_without_environment_trust --test-threads=1
```

The wrapper verified the binary hash before launch, refused an existing attempt
marker, used closed stdin, a minimal PATH and a fresh owned temporary directory,
and imposed the timeout. The request, attempt marker, result and stdout/stderr
are retained beside the executable. [Machine record](native-identity-checks.json)
binds those files and the wrapper by hash.

This evidence covers the selected native identity operations on this host. It
does not qualify Keychain availability, setup transactions, child signalling,
Seatbelt enforcement, tools/checks/hooks, managed services, live providers or
human-reviewed runtime operation. The separate native owned-child test was not
included in the approval or executed. Ordinary test suites continue to ignore
these native cases so a future test invocation cannot silently reuse this action
authorization. The retained alpha.3 artifact and report preserve their original
unrun status; this report records the later supervised result.
