# LightOS instance discovery

Neko discovers LightOS and remote-client instances in the current platform
user's context. The package requests `lzcapp.user_delegate`; each request sends
only its platform-injected `X-HC-User-Ticket` and ordinary content-negotiation
headers to `http://app.cloud.lazycat.lightos.entry.lzcx`. The platform validates
the ticket and projects the user identity for LightOS.

The fixed origin serves `/api/webshell/instances`, `/api/client-instances`, and
`/api/client-instances/terminal-ticket`. Discovery does not depend on the public
admin URL or `LIGHTOS_ADMIN_INTERNAL_BASE_URL`. The delegated client bypasses
environment proxies and rejects redirects. Tickets are bounded, marked
sensitive, and never stored, logged, or returned in error responses. Browser
Cookie/Authorization and incoming identity claims do not cross this boundary.

LightOS supplies the user's visible set, including instances owned by other
applications such as Catflap. Selectors retain `name@owner_deploy_id`, so equal
names with different owners remain distinct. Per-target `lightosctl`
authorization still runs before terminal operations; discovery does not grant
additional access.

Missing or expired delegation, access denial, malformed upstream JSON, and
upstream failures return errors instead of an empty list. Reopen Neko from the
LazyCat platform when the user ticket is unavailable or expired. The official
LightOS embedded mode and development setups that explicitly disable cookie
auth retain their existing admin URL and credential-forwarding path without
receiving a user ticket.

## Default login account

Agent v21 refreshes the workspace's default username from each non-empty,
authorized request. Missing or blank metadata leaves a confirmed username
intact. Only future tabs and splits use the updated default; existing pane
objects, process identities, and sessions are preserved.

The Agent protocol remains v4 and the minimum supported implementation remains
v11. Installing a new provider does not replace a compatible running Agent.
The account-refresh fix takes effect when Agent v21 or later starts; the Agent
update tool can prepare the bundled payload for the next launch without
terminating current PTYs.
