# Batch Actions

Batch actions let you select multiple items and perform an operation on all of them at once.
This is faster than using the context menu on each item individually.

## How to use

1. **Select items** --- check the checkbox next to each item you want to act on, or use the
   checkbox in the table header to select all visible items.
2. **Choose an action** --- a floating toolbar appears at the bottom of the screen showing the
   number of selected items and the available actions.
3. **Confirm** --- destructive actions (delete, deactivate, reject) show a confirmation dialog
   before proceeding.
4. **Review results** --- if some items succeed and others fail, a results dialog shows which
   items failed and why. If all items succeed, a success toast is shown instead.

## Available batch actions by page

| Page | Actions | Permission required |
| --- | --- | --- |
| Services | Approve, Reject, Deactivate | Manage Agents |
| System Services | Approve, Reject, Deactivate | Manage System Services |
| Software | Ignore, Delete | Manage Software |
| Hosts | Deactivate | Manage Hosts |
| Plugin Configs | Delete | Manage Software |
| Host Tags | Delete | Manage Hosts |
| Ignore Rules | Delete | Manage Software |

### Context-dependent actions

Some batch actions are only shown when relevant:

- **Approve** and **Reject** only appear when at least one selected item is in the *Pending*
  state.
- **Deactivate** only appears when at least one selected item is not already deactivated.
- On the Software page, **Ignore** is only shown on the Pending tab.

## Partial success

Batch actions use partial-success semantics. Each item is processed independently --- if one
item fails (e.g. it was already deleted or is in the wrong state), the remaining items are
still processed. The results dialog shows exactly which items succeeded and which failed with
per-item error messages.

## Limits

A single batch request can contain up to 100 items. If you need to act on more items, perform
the batch action in multiple rounds.

## Surface batch actions

Surfaces can mark their actions as batch-capable. When a DataTable surface has
batch-capable actions, the same checkbox and toolbar pattern is available. The surface
receives the IDs of all selected rows and is responsible for processing them.
