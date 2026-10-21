# Security Policy

## Supported versions

Rama is in early development. Security fixes are made on the latest release
only; please update to it before reporting.

## Reporting a vulnerability

**Please do not report security vulnerabilities in public issues.**

Report them privately through GitHub:
[**Report a vulnerability**](https://github.com/yosoyjaff/rama/security/advisories/new).

Include as much as you can:

- what the issue is and what an attacker could do with it;
- the steps or a proof of concept to reproduce it;
- the Rama version (*Settings → About*) and your operating system.

You'll get an acknowledgement as soon as possible. Once the issue is confirmed
and fixed, a release will be published and you'll be credited in the advisory
unless you prefer otherwise.

## Scope

Areas that are especially security-sensitive:

- **Updates** — Rama downloads new versions from this repository's GitHub
  Releases and verifies each archive's size and SHA-256 against the release
  manifest before installing it.
- **Git operations** — Rama runs the `git` executable on repositories you
  open; problems in how it builds those commands are in scope.
- **Credentials** — Rama does not store Git credentials itself; it relies on
  Git's own credential helpers.
