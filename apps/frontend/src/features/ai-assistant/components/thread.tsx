import {
  ActionBarPrimitive,
  BranchPickerPrimitive,
  ComposerPrimitive,
  ErrorPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useAssistantState,
} from "@assistant-ui/react";

import { Component, type ErrorInfo, type FC, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "@wealthfolio/ui/components/ui/button";
import { Icons } from "@wealthfolio/ui/components/ui/icons";
import { ComposerAddAttachment, ComposerAttachments, UserMessageAttachments } from "./attachment";
import { MarkdownText } from "./markdown-text";
import { ToolFallback } from "./tool-fallback";
import { TooltipIconButton } from "./tooltip-icon-button";

import { LIST_ROW } from "@/components/dashboard-card";
import { cn } from "@/lib/utils";
import { Reasoning, ReasoningGroup } from "./reasoning";

export interface ThreadProps {
  composerActions?: ReactNode;
}

export const Thread: FC<ThreadProps> = ({ composerActions }) => {
  const { t } = useTranslation();
  return (
    <ThreadPrimitive.Root
      className="aui-root aui-thread-root bg-background @container flex h-full min-h-0 flex-col"
      style={{
        ["--thread-max-width" as string]: "44rem",
      }}
    >
      <ThreadPrimitive.Viewport
        autoScroll
        turnAnchor="top"
        scrollToBottomOnRunStart
        scrollToBottomOnInitialize
        scrollToBottomOnThreadSwitch
        className="aui-thread-viewport relative flex min-h-0 flex-1 flex-col overflow-x-auto overflow-y-scroll scroll-smooth px-4 pt-4"
      >
        <div className="flex min-h-full flex-col">
          <ThreadPrimitive.Empty>
            <ThreadWelcome />
          </ThreadPrimitive.Empty>

          <ThreadPrimitive.Messages
            components={{
              UserMessage,
              EditComposer,
              AssistantMessage,
            }}
          />
          {/* Spacer to prevent last message from being hidden behind sticky composer + mobile nav */}
          <div className="h-[var(--mobile-nav-total-offset)] shrink-0 md:h-0" />
          <ThreadPrimitive.ViewportFooter className="aui-thread-viewport-footer bg-background max-w-(--thread-max-width) sticky bottom-[var(--mobile-nav-total-offset)] z-10 mx-auto mt-auto flex w-full flex-col gap-4 overflow-visible rounded-t-3xl pb-4 md:bottom-0 md:pb-6">
            <ThreadScrollToBottom />
            <Composer composerActions={composerActions} />
            <p className="text-muted-foreground/70 text-center text-xs">
              {t("ai:thread.disclaimer")}
            </p>
          </ThreadPrimitive.ViewportFooter>
        </div>
      </ThreadPrimitive.Viewport>
    </ThreadPrimitive.Root>
  );
};

const ThreadScrollToBottom: FC = () => {
  const { t } = useTranslation();
  return (
    <ThreadPrimitive.ScrollToBottom behavior="smooth" asChild>
      <TooltipIconButton
        tooltip={t("ai:thread.scrollToBottom")}
        variant="outline"
        className="aui-thread-scroll-to-bottom dark:bg-background dark:hover:bg-accent absolute -top-12 z-10 self-center rounded-full p-4 disabled:invisible"
      >
        <Icons.ArrowDown />
      </TooltipIconButton>
    </ThreadPrimitive.ScrollToBottom>
  );
};

const ThreadWelcome: FC = () => {
  const { t } = useTranslation();
  return (
    <div className="aui-thread-welcome-root max-w-(--thread-max-width) mx-auto flex w-full grow flex-col pt-2">
      <h1 className="fade-in animate-in text-[32px] font-medium leading-tight tracking-tight duration-300 ease-out">
        {t("ai:thread.title")}
      </h1>
      <p className="text-muted-foreground pb-3 pt-1 text-[15px]">{t("ai:thread.subtitle")}</p>
      <ThreadSuggestions />
    </div>
  );
};

/** Things to ask, laid out as the list rows the rest of the app uses. */
const STARTERS = [
  { icon: "Sparkles", key: "categorize" },
  { icon: "Wallet", key: "spending" },
  { icon: "TrendingUp", key: "performance" },
  { icon: "PieChart", key: "allocation" },
] as const;

const ThreadSuggestions: FC = () => {
  const { t } = useTranslation();
  return (
    <div className="aui-thread-welcome-suggestions pb-4">
      {STARTERS.map((starter, index) => {
        const Icon = Icons[starter.icon];
        const title = t(`ai:thread.starters.${starter.key}.title`);
        return (
          <div
            key={starter.key}
            className="fade-in slide-in-from-bottom-2 animate-in fill-mode-both duration-300 ease-out"
            style={{ animationDelay: `${index * 50}ms` }}
          >
            <ThreadPrimitive.Suggestion
              prompt={t(`ai:thread.starters.${starter.key}.prompt`)}
              asChild
            >
              <button type="button" className={cn(LIST_ROW, "w-[calc(100%+1rem)] text-left")}>
                <span className="bg-foreground/[0.07] flex h-10 w-10 shrink-0 items-center justify-center rounded-full">
                  <Icon className="h-[18px] w-[18px]" aria-hidden="true" />
                </span>
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[17px] leading-snug">{title}</span>
                  <span className="text-muted-foreground block truncate text-[15px] leading-snug">
                    {t(`ai:thread.starters.${starter.key}.sub`)}
                  </span>
                </span>
                <Icons.ChevronRight
                  className="text-muted-foreground h-4 w-4 shrink-0"
                  aria-hidden="true"
                />
              </button>
            </ThreadPrimitive.Suggestion>
          </div>
        );
      })}
    </div>
  );
};

interface ComposerProps {
  composerActions?: ReactNode;
}

const Composer: FC<ComposerProps> = ({ composerActions }) => {
  const { t } = useTranslation();
  return (
    <ComposerPrimitive.Root className="aui-composer-root relative flex w-full flex-col">
      <ComposerPrimitive.AttachmentDropzone className="aui-composer-attachment-dropzone glass-pill data-[dragging=true]:bg-accent/50 flex w-full flex-col rounded-[26px] px-1 pt-1.5 outline-none">
        <ComposerAttachments />
        <ComposerPrimitive.Input
          placeholder={t("ai:thread.composerPlaceholder")}
          className="aui-composer-input placeholder:text-muted-foreground max-h-32 min-h-9 w-full resize-none bg-transparent px-3.5 pb-0 pt-1.5 text-base outline-none focus-visible:ring-0"
          rows={1}
          autoFocus
          aria-label={t("ai:thread.messageInput")}
        />
        <ComposerAction composerActions={composerActions} />
      </ComposerPrimitive.AttachmentDropzone>
    </ComposerPrimitive.Root>
  );
};

interface ComposerActionProps {
  composerActions?: ReactNode;
}

const ComposerAction: FC<ComposerActionProps> = ({ composerActions }) => {
  const { t } = useTranslation();
  return (
    <div className="aui-composer-action-wrapper relative mx-1 mb-1.5 mt-0.5 flex items-center justify-between">
      <div className="flex items-center gap-1">
        <ComposerAddAttachment />
        {composerActions}
      </div>

      <ThreadPrimitive.If running={false}>
        <ComposerPrimitive.Send asChild>
          <TooltipIconButton
            tooltip={t("ai:thread.sendMessage")}
            side="bottom"
            type="submit"
            variant="default"
            size="icon"
            className="aui-composer-send size-[34px] rounded-full p-1"
            aria-label={t("ai:thread.sendMessage")}
          >
            <Icons.ArrowUp className="aui-composer-send-icon size-5" />
          </TooltipIconButton>
        </ComposerPrimitive.Send>
      </ThreadPrimitive.If>

      <ThreadPrimitive.If running>
        <ComposerPrimitive.Cancel asChild>
          <Button
            type="button"
            variant="default"
            size="icon"
            className="aui-composer-cancel border-muted-foreground/60 hover:bg-primary/75 dark:border-muted-foreground/90 size-[34px] rounded-full border"
            aria-label={t("ai:thread.stopGenerating")}
          >
            <Icons.Square className="aui-composer-cancel-icon size-3.5 fill-white dark:fill-black" />
          </Button>
        </ComposerPrimitive.Cancel>
      </ThreadPrimitive.If>
    </div>
  );
};

const MessageError: FC = () => {
  return (
    <MessagePrimitive.Error>
      <ErrorPrimitive.Root className="aui-message-error-root border-destructive bg-destructive/10 text-destructive dark:bg-destructive/5 mt-2 rounded-md border p-3 text-sm dark:text-red-200">
        <ErrorPrimitive.Message className="aui-message-error-message line-clamp-2" />
      </ErrorPrimitive.Root>
    </MessagePrimitive.Error>
  );
};

interface MessagePartWithResult {
  type?: string;
  result?: unknown;
}

interface ToolRenderErrorBoundaryState {
  error: Error | null;
}

class ToolRenderErrorBoundary extends Component<
  { children: ReactNode },
  ToolRenderErrorBoundaryState
> {
  override state: ToolRenderErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): ToolRenderErrorBoundaryState {
    return { error };
  }

  override componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("Tool UI render failed", {
      error,
      componentStack: errorInfo.componentStack,
    });
  }

  override render() {
    if (this.state.error) {
      return <ToolRenderErrorFallback />;
    }

    return this.props.children;
  }
}

const ToolRenderErrorFallback: FC = () => {
  const { t } = useTranslation();
  return (
    <div className="border-destructive/30 bg-destructive/5 text-destructive flex items-center gap-2 rounded-md border px-3 py-2 text-xs">
      <Icons.AlertCircle className="h-4 w-4 shrink-0" />
      <span>{t("ai:thread.toolDisplayFailed")}</span>
    </div>
  );
};

interface TypingIndicatorProps {
  position: "initial" | "continuation";
}

const TypingIndicator: FC<TypingIndicatorProps> = ({ position }) => {
  const showIndicator = useAssistantState(({ message }) => {
    if (message.status?.type !== "running") return false;

    if (position === "initial") {
      return message.parts.length === 0;
    }

    const lastPart = message.parts[message.parts.length - 1] as MessagePartWithResult | undefined;
    return (
      lastPart?.type === "tool-call" &&
      Object.prototype.hasOwnProperty.call(lastPart, "result") &&
      lastPart.result !== undefined
    );
  });

  if (!showIndicator) return null;

  return (
    <div className="bg-muted mb-2 inline-flex items-end gap-1 rounded-2xl px-3.5 pb-2.5 pt-3">
      <span className="bg-foreground/50 size-2 animate-bounce rounded-full [animation-delay:-0.3s]" />
      <span className="bg-foreground/50 size-2 animate-bounce rounded-full [animation-delay:-0.15s]" />
      <span className="bg-foreground/50 size-2 animate-bounce rounded-full" />
    </div>
  );
};

const AssistantMessage: FC = () => {
  return (
    <MessagePrimitive.Root
      className="aui-assistant-message-root fade-in slide-in-from-bottom-1 animate-in max-w-(--thread-max-width) relative mx-auto w-full py-4 duration-150 ease-out"
      data-role="assistant"
    >
      <div className="aui-assistant-message-content text-foreground wrap-break-word mx-2 text-sm leading-6">
        <TypingIndicator position="initial" />
        <MessagePrimitive.Parts
          components={{
            Text: MarkdownText,
            Reasoning: Reasoning,
            ReasoningGroup: ReasoningGroup,
            ToolGroup: ({ children }) => (
              <ToolRenderErrorBoundary>
                <div className="mb-6 space-y-4">{children}</div>
              </ToolRenderErrorBoundary>
            ),
            tools: {
              Fallback: ToolFallback,
            },
          }}
        />
        <TypingIndicator position="continuation" />
        <MessageError />
      </div>

      <div className="aui-assistant-message-footer ml-2 mt-2 flex">
        <BranchPicker />
        <AssistantActionBar />
      </div>
    </MessagePrimitive.Root>
  );
};

const AssistantActionBar: FC = () => {
  const { t } = useTranslation();
  return (
    <ActionBarPrimitive.Root
      hideWhenRunning
      autohide="not-last"
      autohideFloat="single-branch"
      className="aui-assistant-action-bar-root text-muted-foreground data-floating:bg-background data-floating:absolute data-floating:rounded-md data-floating:border data-floating:p-1 data-floating:shadow-sm col-start-3 row-start-2 -ml-1 flex gap-1"
    >
      <ActionBarPrimitive.Copy asChild>
        <TooltipIconButton tooltip={t("ai:thread.copy")}>
          <MessagePrimitive.If copied>
            <Icons.Check />
          </MessagePrimitive.If>
          <MessagePrimitive.If copied={false}>
            <Icons.Copy />
          </MessagePrimitive.If>
        </TooltipIconButton>
      </ActionBarPrimitive.Copy>
      <ActionBarPrimitive.Reload asChild>
        <TooltipIconButton tooltip={t("ai:thread.refresh")}>
          <Icons.RefreshCw />
        </TooltipIconButton>
      </ActionBarPrimitive.Reload>
    </ActionBarPrimitive.Root>
  );
};

const UserMessage: FC = () => {
  return (
    <MessagePrimitive.Root
      className="aui-user-message-root fade-in slide-in-from-bottom-1 animate-in max-w-(--thread-max-width) mx-auto grid w-full auto-rows-auto grid-cols-[minmax(72px,1fr)_auto] content-start gap-y-2 px-2 py-4 duration-150 ease-out [&:where(>*)]:col-start-2"
      data-role="user"
    >
      <UserMessageAttachments />

      <div className="aui-user-message-content-wrapper relative col-start-2 min-w-0">
        <div className="aui-user-message-content bg-foreground/[0.07] text-foreground wrap-break-word rounded-3xl px-5 py-2.5 text-[15px]">
          <MessagePrimitive.Parts
            components={{
              Reasoning: Reasoning,
              ReasoningGroup: ReasoningGroup,
            }}
          />
        </div>
        <div className="aui-user-action-bar-wrapper absolute left-0 top-1/2 -translate-x-full -translate-y-1/2 pr-2">
          <UserActionBar />
        </div>
      </div>

      <BranchPicker className="aui-user-branch-picker col-span-full col-start-1 row-start-3 -mr-1 justify-end" />
    </MessagePrimitive.Root>
  );
};

const UserActionBar: FC = () => {
  const { t } = useTranslation();
  return (
    <ActionBarPrimitive.Root
      hideWhenRunning
      autohide="not-last"
      className="aui-user-action-bar-root flex flex-col items-end"
    >
      <ActionBarPrimitive.Edit asChild>
        <TooltipIconButton tooltip={t("ai:thread.edit")} className="aui-user-action-edit p-4">
          <Icons.Pencil />
        </TooltipIconButton>
      </ActionBarPrimitive.Edit>
    </ActionBarPrimitive.Root>
  );
};

const EditComposer: FC = () => {
  const { t } = useTranslation();
  return (
    <MessagePrimitive.Root className="aui-edit-composer-wrapper max-w-(--thread-max-width) mx-auto flex w-full flex-col gap-4 px-2">
      <ComposerPrimitive.Root className="aui-edit-composer-root bg-muted max-w-7/8 ml-auto flex w-full flex-col rounded-xl">
        <ComposerPrimitive.Input
          className="aui-edit-composer-input text-foreground flex min-h-[60px] w-full resize-none bg-transparent p-4 outline-none"
          autoFocus
        />

        <div className="aui-edit-composer-footer mx-3 mb-3 flex items-center justify-center gap-2 self-end">
          <ComposerPrimitive.Cancel asChild>
            <Button variant="ghost" size="sm" aria-label={t("ai:thread.cancelEdit")}>
              {t("ai:thread.cancel")}
            </Button>
          </ComposerPrimitive.Cancel>
          <ComposerPrimitive.Send asChild>
            <Button size="sm" aria-label={t("ai:thread.updateMessage")}>
              {t("ai:thread.update")}
            </Button>
          </ComposerPrimitive.Send>
        </div>
      </ComposerPrimitive.Root>
    </MessagePrimitive.Root>
  );
};

const BranchPicker: FC<BranchPickerPrimitive.Root.Props> = ({ className, ...rest }) => {
  const { t } = useTranslation();
  return (
    <BranchPickerPrimitive.Root
      hideWhenSingleBranch
      className={cn(
        "aui-branch-picker-root text-muted-foreground -ml-2 mr-2 inline-flex items-center text-xs",
        className,
      )}
      {...rest}
    >
      <BranchPickerPrimitive.Previous asChild>
        <TooltipIconButton tooltip={t("ai:thread.previous")}>
          <Icons.ChevronLeft />
        </TooltipIconButton>
      </BranchPickerPrimitive.Previous>
      <span className="aui-branch-picker-state font-medium">
        <BranchPickerPrimitive.Number /> / <BranchPickerPrimitive.Count />
      </span>
      <BranchPickerPrimitive.Next asChild>
        <TooltipIconButton tooltip={t("ai:thread.next")}>
          <Icons.ChevronRight />
        </TooltipIconButton>
      </BranchPickerPrimitive.Next>
    </BranchPickerPrimitive.Root>
  );
};
