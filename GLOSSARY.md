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
A reference from a page or note to a page; a reference to a source is a citation instead.
_Avoid_: Mention, edge, reference

**Note**:
A short, timestamped, append-only entry recording something worth remembering, which links to the pages it concerns.
_Avoid_: Memory, memo, log entry

**Revision**:
One saved state of a page; every change to a page creates a new revision.
_Avoid_: Version, history entry

**Author**:
The owner or agent that created a revision or note, as established by authentication; for the built-in agent's writes, the author is the built-in agent.
_Avoid_: Creator, editor, source

### Sources

**Source**:
An external piece of content, an uploaded file or a web resource, kept with its metadata, extracted text and abstract. Unlike a page, it is not curated by revisions.
_Avoid_: Document, reference, attachment, external link

**Snapshot**:
The unchanging content of a source as it was uploaded or fetched at one time; refreshing a web source adds a new snapshot when its content changed.
_Avoid_: Version, revision, copy

**Abstract**:
A short summary of a source's content.
_Avoid_: Summary, description

**Citation**:
A reference from a page or note to a source.
_Avoid_: Link, reference, footnote

**Import**:
Adding a source and working its content into the knowledge: abstract, page changes and notes citing it.
_Avoid_: Upload, ingestion

### Built-in agent work

**Conversation**:
An exchange between the owner and the built-in agent in the web UI.
_Avoid_: Chat session, thread

**Cause**:
The import or conversation that led the built-in agent to a write; everything with the same cause can be reverted together.
_Avoid_: Origin, trigger, batch

### Actors

**Owner**:
The single human who runs mnemograph and the only one allowed to delete anything.
_Avoid_: Admin, user

**Agent**:
One individually registered AI client installation (for example Claude Code on a particular machine) that reads and writes knowledge through MCP. It can create and change pages and notes, but never delete them.
_Avoid_: Bot, client, assistant

**Built-in agent**:
The agent that is part of mnemograph itself, backed by an LLM; it imports sources and holds conversations with the owner, under the same rules as every other agent.
_Avoid_: Assistant, system, service
