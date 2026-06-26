import { commandsMatch } from "@/features/agents/agentReuse";
import type { ParsePersonaFilesResult } from "@/shared/api/tauriPersonas";
import type {
  AcpRuntimeCatalogEntry,
  AgentPersona,
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";

export type PersonaDialogState = {
  description: string;
  initialValues: CreatePersonaInput | UpdatePersonaInput;
  submitLabel: string;
  title: string;
};

type ParsedPersonaDraft = ParsePersonaFilesResult["personas"][number];

export function createPersonaDialogState(): PersonaDialogState {
  return {
    title: "Create persona",
    description:
      "Save a reusable role, prompt, and optional avatar for future agent deployments.",
    submitLabel: "Create persona",
    initialValues: {
      displayName: "",
      avatarUrl: "",
      systemPrompt: "",
      runtime: undefined,
      model: undefined,
    },
  };
}

export function duplicatePersonaDialogState(
  persona: AgentPersona,
): PersonaDialogState {
  return {
    title: `Duplicate ${persona.displayName}`,
    description:
      "Create a new persona by copying this template and adjusting it as needed.",
    submitLabel: "Create persona",
    initialValues: {
      displayName: `${persona.displayName} copy`,
      avatarUrl: persona.avatarUrl ?? "",
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      // Carry envVars and namePool into the duplicate. Without this, a
      // duplicated persona that relies on an API key in env_vars would
      // silently fail at spawn until the user re-entered every credential.
      // The user sees the inherited values in the dialog and can clear
      // them if they want a blank template.
      namePool: persona.namePool ?? [],
      envVars: persona.envVars ?? {},
    },
  };
}

/**
 * Reverse-map a managed agent's resolved harness command back to an ACP
 * runtime ID, so the persona dialog can pre-select the matching runtime.
 * Returns `undefined` when no runtime matches (or none are loaded yet) — the
 * dialog then falls back to its default-runtime behavior.
 */
function runtimeIdForAgentCommand(
  agentCommand: string,
  runtimes: readonly AcpRuntimeCatalogEntry[],
): string | undefined {
  const match = runtimes.find(
    (runtime) =>
      runtime.command !== null && commandsMatch(runtime.command, agentCommand),
  );
  return match?.id;
}

/**
 * Dialog state for the opt-in "Save as persona template" action on an existing
 * agent. Prefills the persona editor from the agent so the user reviews and
 * confirms before a persona template is created — nothing is minted silently.
 *
 * Near-lossless promote: name, system prompt, model, and env vars copy
 * straight across; the harness command reverse-maps to a runtime ID.
 * `namePool` is persona-only and starts empty — the user can fill it in the
 * same dialog (it's how a template bulk-adds bots later).
 *
 * Note: "persona template" is the UI name for what the backend calls a
 * `persona` (kind:30175). This builder produces a backend `CreatePersonaInput`.
 * `provider` is intentionally not carried: a `ManagedAgent` does not expose a
 * top-level provider field (it lives in backend/provider config), so there is
 * nothing lossless to copy here.
 */
export function saveAsPersonaTemplateDialogState(
  agent: ManagedAgent,
  runtimes: readonly AcpRuntimeCatalogEntry[],
): PersonaDialogState {
  return {
    title: "Save as persona template",
    description: "Reuse this setup to create more agents.",
    submitLabel: "Save as persona template",
    initialValues: {
      displayName: agent.name,
      avatarUrl: "",
      systemPrompt: agent.systemPrompt ?? "",
      runtime: runtimeIdForAgentCommand(agent.agentCommand, runtimes),
      model: agent.model ?? undefined,
      // namePool is persona-only; start empty so the user fills it here.
      namePool: [],
      envVars: agent.envVars ?? {},
    },
  };
}

export function editPersonaDialogState(
  persona: AgentPersona,
): PersonaDialogState {
  return {
    title: "Edit persona",
    description: "",
    submitLabel: "Save changes",
    initialValues: {
      id: persona.id,
      displayName: persona.displayName,
      avatarUrl: persona.avatarUrl ?? "",
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
      // Seed both namePool and envVars from the loaded persona so editing
      // unrelated fields doesn't submit an empty value that wipes them.
      // (Persona update treats Some(empty) as "clear all" intentionally;
      // the dialog must therefore round-trip the existing values.)
      namePool: persona.namePool ?? [],
      envVars: persona.envVars ?? {},
    },
  };
}

export function importPersonaDialogState(
  persona: ParsedPersonaDraft,
): PersonaDialogState {
  return {
    title: `Import ${persona.displayName}`,
    description: "Review and save this imported persona.",
    submitLabel: "Create persona",
    initialValues: {
      displayName: persona.displayName,
      avatarUrl: persona.avatarDataUrl ?? "",
      systemPrompt: persona.systemPrompt,
      runtime: persona.runtime ?? undefined,
      model: persona.model ?? undefined,
      provider: persona.provider ?? undefined,
    },
  };
}
