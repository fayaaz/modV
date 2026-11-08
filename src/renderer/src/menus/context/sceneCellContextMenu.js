/**
 * Right-click menu for a performance-grid cell. Editing loads the cell's Group
 * (applying that Scene row if needed) and clearing removes the Group's Modules
 * after a confirmation.
 */
export function SceneCellContextMenu({
  scene,
  group,
  rowIndex,
  editEnabled = true,
  clearEnabled = true,
  onEdit,
  onClear,
}) {
  const groupName = (group && group.name) || `Row ${rowIndex + 1}`;

  return [
    {
      label: `${scene.name} · ${groupName}`,
      enabled: false,
    },
    {
      type: "separator",
    },
    {
      label: "Edit",
      enabled: editEnabled,
      click() {
        return onEdit && onEdit();
      },
    },
    {
      label: "Clear",
      enabled: clearEnabled,
      click() {
        return onClear && onClear();
      },
    },
  ];
}
