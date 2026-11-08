# Shared spheres are read by every connection and changed only through their own

Some pages concern every sphere: above all the page with the owner's filing conventions ("Using mnemograph"), which every agent should read before writing. Under ADR 0008 such a page lies in some sphere and is invisible to connections of the others, and since each connection only sets spheres among its own, no agent can make it visible everywhere. So a sphere rule `readable_by = "all"` marks a **shared sphere**: every connection sees its pages and notes besides those of its own spheres (reading, search, links, related pages, the usage hint), but only connections that have the sphere as their own create pages and notes in it, change its pages or add it to a page. A note of another sphere may link to a shared page and stays in its own sphere, hidden from the shared sphere's connection. The owner's configuration names its shared sphere `meta`.

## Considered Options

- **Every agent may change the shared page:** convenient, but any connection could then carry content from its sphere into all others, deliberately or through prompt injection; a shared page would become a channel between spheres.
- **A special case for the usage page only** (visible everywhere, owner-only writes): no new rule, but the owner has no way to write it except through an agent, and other shared pages would need the same special case again.
- **Conventions in the central configuration, sent as instructions:** no sphere question at all, but they could no longer be edited and linked like other pages, and the instructions would grow long.

## Consequences

Content flows only from a shared sphere into the others, never back or across. Visibility for reading (own and shared spheres) and for changing (own spheres) are now different sets, so writes check both. Nothing specific to one sphere belongs in a shared sphere; the instructions say so, but only the owner's discipline and the connection's binding to a repository keep it that way. Feedback from other spheres reaches a shared page as notes in their own spheres, which the owner sees.
