# Security policy

## Supported versions

| Version | Supported |
| --- | --- |
| 1.0.x | Yes |

Security fixes go into the latest release only.

## Reporting a vulnerability

Please report security problems privately, not in a public issue: open the **Security** tab of this repository and choose **Report a vulnerability**. Examples are log text that can add a fake line or a terminal escape to the output, a secret that is shown unmasked while redaction is on, a way to make symlog write, run or hang on something it should refuse, or a crash on a malformed log file.

Please include:

- the symlog version (`symlog --version`) and your operating system;
- the exact command;
- a minimal synthetic log file that reproduces the problem, never a real log;
- what symlog printed and what you expected.

We aim to respond within 7 days (best effort). Once a fix is released, the report can be credited in the changelog if you wish.
