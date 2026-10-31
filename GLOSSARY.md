# mnemograph

A self-hosted knowledge base that is both a human-readable wiki and a long-term memory for AI agents.

## Language

### Knowledge

**Page**:
A curated, evolving unit of knowledge about one subject, identified by its title. Anything worth linking to is a page.
_Avoid_: Document, entity, article

**Alias**:
An alternative title that leads to the same page; a renamed page keeps its old title as an alias.
_Avoid_: Redirect, synonym

**Stub**:
A title that links point to but that no page has yet; it becomes a page the first time someone writes it.
_Avoid_: Placeholder, missing page

**Page type**:
An optional classification of a page, chosen from a fixed list defined by the owner.
_Avoid_: Entity type, category

**Link**:
A reference from a page or note to a page; a reference to another service's item is a citation instead.
_Avoid_: Mention, edge, reference

**Note**:
A short, timestamped, append-only entry recording something worth remembering, which links to the pages it concerns.
_Avoid_: Memory, memo, log entry

**Revision**:
One saved state of a page; every change to a page creates a new revision.
_Avoid_: Version, history entry

**Author**:
The owner or agent that created a revision or note, as established by authentication.
_Avoid_: Creator, editor, source

### Spheres

**Sphere**:
An area of the owner's life with its own privacy boundary, such as work, private or a club; every page and note belongs to one or more.
_Avoid_: Domain, space, tenant, category

**Citation**:
A reference from a page or note to an item kept by another service of the owner's assistant (a document, an appointment), made by that item's address.
_Avoid_: Link (a link points to a page), attachment, source

### Actors

**Owner**:
The single human who runs mnemograph and the only one allowed to delete anything.
_Avoid_: Admin, user

**Agent**:
One registered AI client installation for a set of spheres (for example Claude Code on a particular machine, for work) that reads and writes knowledge through MCP. It sees only pages and notes of its spheres, can create and change them, but never delete them.
_Avoid_: Bot, client, assistant
