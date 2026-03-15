1. After ID shortening lands, re-measure the natural column widths.
2. Use terminal width detection (term_size or similar) to dynamically allocate column space.
3. Priority order: ID > STATUS > TITLE > PRI > TYPE. Title gets remaining width.
4. Truncate title with '...' rather than wrapping if insufficient space remains.
5. If terminal width is unavailable (piped output), use a sensible fixed width (e.g. 120 chars total).