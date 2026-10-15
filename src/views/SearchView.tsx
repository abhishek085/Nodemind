// Search: find anything you've said, by meaning (EmbeddingGemma 2).

import { SearchInput, SearchResults, useSearch } from "../components/search";

export default function SearchView({ onShowInMap }: { onShowInMap: (thoughtId: string) => void }) {
  const s = useSearch(20);
  return (
    <div className="flex-1 overflow-y-auto p-6">
      <div className="mx-auto max-w-2xl">
        <SearchInput value={s.query} onChange={s.setQuery} busy={s.busy} autoFocus />
        {s.query.trim().length < 2 && (
          <p className="mt-4 text-xs leading-relaxed text-ink-3">
            Search the ideas, questions, decisions, learnings and tasks you've said — "pricing",
            "what did I promise Rahul", "app ideas". It matches by meaning, so the words don't
            have to be the ones you used. <kbd className="font-mono">⌘K</kbd> opens this from
            anywhere in the app.
          </p>
        )}
        <SearchResults hits={s.hits} query={s.query} error={s.error} onOpen={(h) => onShowInMap(h.id)} />
      </div>
    </div>
  );
}
