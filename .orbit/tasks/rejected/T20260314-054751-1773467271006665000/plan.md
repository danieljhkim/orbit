1. Locate search implementation in orbit-core/src/command/task.rs or orbit-store.
2. Determine what fields are currently indexed/searched (likely description/plan only, not title).
3. Add title to the search corpus.
4. Add a test: create a task with a distinctive title, search for a word from the title, assert it appears in results.
5. Also verify search matches on partial words (e.g. 'shorten' matches 'Shorten').