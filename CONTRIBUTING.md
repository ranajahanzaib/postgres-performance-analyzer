# Contributing

Contributions that improve correctness, safety, documentation, and usability are welcome. For substantial changes, open an issue first to discuss the approach.

## Development

1. Fork and clone the repository.
2. Create a focused branch for the change.
3. Make the change and add or update tests where appropriate.
4. Run the checks listed in [Development and CI](README.md#development-and-ci).
5. Open a pull request with a concise description, motivation, and any relevant limitations.

Follow the existing Rust formatting and documentation style. Do not include real database credentials, private SQL, or production data in commits, fixtures, logs, or issue reports. Use the disposable Compose environment for demonstrations.

Please read the [Code of Conduct](CODE_OF_CONDUCT.md). Report security vulnerabilities according to the [Security Policy](.github/SECURITY.md), rather than in a public issue.
