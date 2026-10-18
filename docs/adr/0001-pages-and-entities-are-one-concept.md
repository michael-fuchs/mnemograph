# Pages and entities are one concept

The initial plan kept documents and entities as separate graph node types, linked by `DESCRIBES`/`MENTIONS`. We merged them: there is only the **Page**, which can carry an optional page type and can exist as a stub without a body. Every link targets a page. With two node types, every link would raise the question of whether it points to the document or the entity, and the schema and MCP tool surface would be twice as large. The Obsidian/Wikipedia model shows that one linkable unit is enough for a personal wiki.
