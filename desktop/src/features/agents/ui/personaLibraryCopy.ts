/**
 * UI copy for the persona library surfaces.
 *
 * ── Vocabulary boundary (UI ↔ backend) ──────────────────────────────────────
 * The product-facing term is **"persona template"**: a reusable setup a user
 * can save once and reuse to create more agents. The backend has no separate
 * "template" concept — a persona template IS a **`persona`** (Nostr
 * **kind:30175**). There is intentionally zero drift between the two:
 *
 *   UI string                backend entity / mutation
 *   ───────────────────────  ────────────────────────────────────────────────
 *   "persona template"   ⇄   `persona` (kind:30175)
 *   "Save as persona     ⇄   `createPersonaMutation` / `CreatePersonaInput`
 *    template"
 *
 * So every "...persona template" label here, in `CreateAgentDialog`'s opt-in
 * toggle, and in the `useSaveAsPersonaTemplate` / `saveAsPersonaTemplateDialogState`
 * save-as flow maps to the same backend `persona`. Keep new persona-template
 * copy in this file and keep the mapping above current if the vocabulary moves.
 */
export const personaLibraryCopy = {
  title: "My agents",
  description:
    "The personas you have chosen for this app. Use them to create teams and launch agents.",
  chooseFromCatalog: "Choose...",
  createNew: "Persona",
  import: "Import",
  emptyTitle: "No agents yet",
  emptyDescription:
    "Choose one from Persona Catalog, add your own persona, or import one to get started.",
  emptyImportHint:
    "Or drop a .persona.md, .persona.json, .persona.png, or .zip file here to import.",
} as const;

export const personaCatalogCopy = {
  title: "Persona Catalog",
  description: "Choose which built-in personas belong in My Agents.",
  dialogTitle: "Choose from Persona Catalog",
  dialogDescription:
    "Select the built-in personas you want available in My Agents.",
  emptyTitle: "You're all set",
  emptyDescription: "Everything in Persona Catalog is already in My Agents.",
  emptyCatalogDescription:
    "New personas will show up here when the app ships more options.",
  emptyCatalogTitle: "No personas in the catalog yet",
  detailsAction: "View details",
  selectAction: "Choose",
  deselectAction: "Selected",
  selectedState: "Selected",
  availableState: "Available",
  detailSelectedTitle: "Selected for My Agents",
  detailSelectedDescription:
    "Turn this off to remove the persona from teams and agent creation in this app.",
  detailAvailableTitle: "Available in Persona Catalog",
  detailAvailableDescription:
    "Turn this on to make the persona available for teams and agent creation.",
  teamEmptyState:
    "No personas in My Agents yet. Create one or choose one from Persona Catalog first.",
} as const;

export function getPersonaCatalogSelectionActionCopy(isActive: boolean) {
  return isActive
    ? personaCatalogCopy.deselectAction
    : personaCatalogCopy.selectAction;
}

export function getPersonaCatalogSelectionAriaLabel(
  displayName: string,
  isActive: boolean,
) {
  return `${isActive ? "Deselect" : "Select"} ${displayName} in My Agents`;
}

export function getPersonaCatalogDetailSelectionCopy(isActive: boolean) {
  return isActive
    ? {
        title: personaCatalogCopy.detailSelectedTitle,
        description: personaCatalogCopy.detailSelectedDescription,
      }
    : {
        title: personaCatalogCopy.detailAvailableTitle,
        description: personaCatalogCopy.detailAvailableDescription,
      };
}
