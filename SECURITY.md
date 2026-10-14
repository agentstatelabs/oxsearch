# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's [private vulnerability reporting](https://github.com/agentstatelabs/oxsearch/security/advisories/new) for this repository. Don't open a public issue.

We aim to acknowledge reports within three working days, and to agree a disclosure date with you once a fix is ready.

## Scope

oxsearch reads input from outside the process: responses from Elasticsearch, and the URLs, credentials and configuration an application gives it. Bugs that let that input crash, stall or exhaust the memory of a program using oxsearch are in scope, as is any way oxsearch could leak credentials, or record data in telemetry that it promises not to.

## Supported versions

Until 1.0, only the latest release receives fixes.
