export function smartRenamePreview(
  enabled: boolean,
  projectFile: string | null,
  suggestedName: string | null | undefined,
): string | null {
  if (!projectFile) return null;
  return enabled ? (suggestedName || null) : projectFile.replace(/\.flp$/i, '');
}
