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
A reference from a page or note to a page.
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

### Actors

**Owner**:
The single human who runs mnemograph and the only one allowed to delete anything.
_Avoid_: Admin, user

**Agent**:
One individually registered AI client installation (for example Claude Code on a particular machine) that reads and writes knowledge through MCP. It can create and change pages and notes, but never delete them.
_Avoid_: Bot, client, assistant
