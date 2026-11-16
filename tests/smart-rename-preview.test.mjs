import test from 'node:test';
import assert from 'node:assert/strict';
import { smartRenamePreview } from '../src/smart-rename-preview.ts';

test('disabled preview preserves the original project name', () => {
  assert.equal(smartRenamePreview(false, 'night 140 F# @producer.FLP', 'changed'), 'night 140 F# @producer');
});

test('enabled preview uses only an actual suggestion and never invents one', () => {
  assert.equal(smartRenamePreview(true, 'night.flp', 'night 140 F# @producer'), 'night 140 F# @producer');
  assert.equal(smartRenamePreview(true, 'night.flp', null), null);
  assert.equal(smartRenamePreview(true, null, 'night 140'), null);
});
