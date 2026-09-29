# Security policy

Please report vulnerabilities privately through GitHub's security advisory
reporting for this repository, when enabled. If private reporting is unavailable,
open an issue requesting a private contact **without exploit details, tokens or
private data**. Do not post a live MCP endpoint, full binding key or reply body.

Include the affected commit/package, OS, reproducible steps using dummy data,
impact and whether sandboxing is available. We cannot promise a response SLA.

See `docs/sessiond/THREAT_MODEL.md`. Same UID alone is not isolation. Trusted
installed code must be root-owned. Local plugin approval verifies package
identity; it does not make a plugin safe or grant ChatGPT authority. Sessiond
never stores reply credentials or chat text on disk. Logs/diagnostics must remain
sanitized. A daemon restart invalidates credentials and never implies a reply
was delivered. The current development series receives security fixes; older
snapshots have no maintenance guarantee.
