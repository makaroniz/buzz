import * as React from "react";
import { toast } from "sonner";

import {
  useAcpRuntimesQuery,
  useCreatePersonaMutation,
} from "@/features/agents/hooks";
import type {
  CreatePersonaInput,
  ManagedAgent,
  UpdatePersonaInput,
} from "@/shared/api/types";
import {
  saveAsPersonaTemplateDialogState,
  type PersonaDialogState,
} from "./personaDialogState";

/**
 * Self-contained "Save as persona template" flow for surfaces that don't
 * already host `usePersonaActions` (e.g. an agent row's actions menu or the
 * sidebar agent profile). Opens the shared `PersonaDialog` prefilled from an
 * agent and creates a backend persona on submit — no new backend or IPC.
 *
 * Keeping the dialog + mutation behind this hook (rather than welding them to
 * a host component's layout) means a later visual reskin of the create surface
 * can move the trigger without untangling the save-as behavior.
 *
 * "Persona template" is the UI name for what the backend calls a `persona`
 * (kind:30175); this hook produces a `CreatePersonaInput`.
 */
export function useSaveAsPersonaTemplate() {
  // Hold the agent being promoted (rather than a one-shot snapshot) so the
  // runtime reverse-map can re-resolve when the runtime list arrives — the
  // query is lazy (`enabled` flips on open), so its data is usually empty on
  // the very first open. Deriving the dialog state from `agent` + live
  // runtimes via a memo avoids that first-open race.
  const [agent, setAgent] = React.useState<ManagedAgent | null>(null);
  // Only fetch runtimes once the user actually opens the dialog.
  const acpRuntimesQuery = useAcpRuntimesQuery({ enabled: agent !== null });
  const createPersonaMutation = useCreatePersonaMutation();

  const dialogState = React.useMemo<PersonaDialogState | null>(
    () =>
      agent
        ? saveAsPersonaTemplateDialogState(agent, acpRuntimesQuery.data ?? [])
        : null,
    [agent, acpRuntimesQuery.data],
  );

  const open = React.useCallback((nextAgent: ManagedAgent) => {
    setAgent(nextAgent);
  }, []);

  const close = React.useCallback(() => {
    setAgent(null);
  }, []);

  const handleSubmit = React.useCallback(
    async (input: CreatePersonaInput | UpdatePersonaInput) => {
      // The save-as flow only ever produces a create input.
      if ("id" in input) return;
      try {
        await createPersonaMutation.mutateAsync(input);
        toast.success(`Saved ${input.displayName} as a persona template.`);
        setAgent(null);
      } catch (error) {
        toast.error(
          error instanceof Error
            ? error.message
            : "Failed to save persona template.",
        );
      }
    },
    [createPersonaMutation],
  );

  return {
    open,
    dialogState,
    dialogProps: {
      open: dialogState !== null,
      title: dialogState?.title ?? "",
      description: dialogState?.description ?? "",
      submitLabel: dialogState?.submitLabel ?? "",
      initialValues: dialogState?.initialValues ?? null,
      error:
        createPersonaMutation.error instanceof Error
          ? createPersonaMutation.error
          : null,
      isPending: createPersonaMutation.isPending,
      runtimes: acpRuntimesQuery.data ?? [],
      runtimesLoading: acpRuntimesQuery.isLoading,
      onOpenChange: (next: boolean) => {
        if (!next) close();
      },
      onSubmit: handleSubmit,
    },
  };
}
