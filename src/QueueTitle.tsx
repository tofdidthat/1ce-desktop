import { useRef, useState } from 'react';

export function QueueTitle({ value, disabled, onSave }: { value: string; disabled: boolean; onSave: (value: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const cancelled = useRef(false);
  function finish() {
    // Focusing a title is not an edit. Otherwise a blur (for example when
    // enabling Smart Rename) freezes the old title as a manual override.
    if (!cancelled.current && draft.trim() && draft.trim() !== value.trim()) onSave(draft.trim());
    setEditing(false);
  }
  return editing ? <input className="queue-title-input" aria-label="Beat name" autoFocus value={draft} maxLength={180}
    onFocus={event => event.currentTarget.select()} onChange={event => setDraft(event.target.value)} onBlur={finish}
    onKeyDown={event => {
      if (event.key === 'Enter') { event.preventDefault(); event.currentTarget.blur(); }
      if (event.key === 'Escape') { event.preventDefault(); cancelled.current = true; setEditing(false); }
    }} /> : <button type="button" className="queue-title-button" disabled={disabled} title={`Edit name: ${value}`}
    aria-label={`Edit name: ${value}`} onClick={() => { cancelled.current = false; setDraft(value); setEditing(true); }}>{value}</button>;
}
