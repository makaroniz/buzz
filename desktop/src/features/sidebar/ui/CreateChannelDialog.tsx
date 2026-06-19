import {
  ChannelCreateForm,
  type ChannelCreateInput,
  type ChannelCreateKind,
} from "@/features/channels/ui/ChannelCreateForm";
import { ChooserDialogContent } from "@/shared/ui/chooser-dialog-content";
import { Dialog } from "@/shared/ui/dialog";

type CreateChannelDialogProps = {
  /** Which kind of channel to create, or null when closed. */
  channelKind: ChannelCreateKind | null;
  isCreating: boolean;
  onOpenChange: (open: boolean) => void;
  onCreate: (input: ChannelCreateInput) => Promise<void>;
};

export function CreateChannelDialog({
  channelKind,
  isCreating,
  onOpenChange,
  onCreate,
}: CreateChannelDialogProps) {
  const open = channelKind !== null;
  const kindLabel = channelKind === "forum" ? "forum" : "channel";

  return (
    <Dialog
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen && isCreating) return;
        onOpenChange(nextOpen);
      }}
    >
      {channelKind ? (
        <ChooserDialogContent
          className="max-w-lg"
          contentClassName="pt-3"
          data-testid="create-channel-dialog"
          footerClassName="border-t-0 pt-0"
          headerClassName="pb-2"
          title={`Create a new ${kindLabel}`}
          description={
            channelKind === "forum"
              ? "Forums organize threaded discussions around a topic."
              : "Channels are real-time streams for team conversation."
          }
        >
          <ChannelCreateForm
            channelKind={channelKind}
            isCreating={isCreating}
            onCreate={onCreate}
            onCreated={() => onOpenChange(false)}
          />
        </ChooserDialogContent>
      ) : null}
    </Dialog>
  );
}
