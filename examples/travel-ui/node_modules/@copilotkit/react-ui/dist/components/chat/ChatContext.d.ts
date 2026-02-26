import * as react_jsx_runtime from 'react/jsx-runtime';
import React__default from 'react';

/**
 * Icons for CopilotChat component.
 */
interface CopilotChatIcons {
    /**
     * The icon to use for the open chat button.
     * @default <OpenIcon />
     */
    openIcon?: React__default.ReactNode;
    /**
     * The icon to use for the close chat button.
     * @default <CloseIcon />
     */
    closeIcon?: React__default.ReactNode;
    /**
     * The icon to use for the close chat button in the header.
     * @default <HeaderCloseIcon />
     */
    headerCloseIcon?: React__default.ReactNode;
    /**
     * The icon to use for the send button.
     * @default <SendIcon />
     */
    sendIcon?: React__default.ReactNode;
    /**
     * The icon to use for the activity indicator.
     * @default <ActivityIcon />
     */
    activityIcon?: React__default.ReactNode;
    /**
     * The icon to use for the spinner.
     * @default <SpinnerIcon />
     */
    spinnerIcon?: React__default.ReactNode;
    /**
     * The icon to use for the stop button.
     * @default <StopIcon />
     */
    stopIcon?: React__default.ReactNode;
    /**
     * The icon to use for the regenerate button.
     * @default <RegenerateIcon />
     */
    regenerateIcon?: React__default.ReactNode;
    /**
     * The icons to use for push to talk.
     * @default <PushToTalkIcon />
     */
    pushToTalkIcon?: React__default.ReactNode;
    /**
     * The icons to use for copy assistant response
     * @default <CopyIcon />
     */
    copyIcon?: React__default.ReactNode;
    /**
     * The icon to use for thumbs up/response approval.
     * @default <ThumbsUpIcon />
     */
    thumbsUpIcon?: React__default.ReactNode;
    /**
     * The icon to use for thumbs down/response rejection.
     * @default <ThumbsDownIcon />
     */
    thumbsDownIcon?: React__default.ReactNode;
    /**
     * The icon to use for the upload button.
     * @default <UploadIcon />
     */
    uploadIcon?: React__default.ReactNode;
}
/**
 * Labels for CopilotChat component.
 */
interface CopilotChatLabels {
    /**
     * The initial message(s) to display in the chat window.
     */
    initial?: string | string[];
    /**
     * The title to display in the header.
     * @default "CopilotKit"
     */
    title?: string;
    /**
     * The placeholder to display in the input.
     * @default "Type a message..."
     */
    placeholder?: string;
    /**
     * The message to display when an error occurs.
     * @default "❌ An error occurred. Please try again."
     */
    error?: string;
    /**
     * The label to display on the stop button.
     * @default "Stop generating"
     */
    stopGenerating?: string;
    /**
     * The label to display on the regenerate button.
     * @default "Regenerate response"
     */
    regenerateResponse?: string;
    /**
     * The label for the copy button.
     * @default "Copy to clipboard"
     */
    copyToClipboard?: string;
    /**
     * The label for the thumbs up button.
     * @default "Thumbs up"
     */
    thumbsUp?: string;
    /**
     * The label for the thumbs down button.
     * @default "Thumbs down"
     */
    thumbsDown?: string;
    /**
     * The text to display when content is copied.
     * @default "Copied!"
     */
    copied?: string;
}
interface ChatContext {
    labels: Required<CopilotChatLabels>;
    icons: Required<CopilotChatIcons>;
    open: boolean;
    setOpen: (open: boolean) => void;
}
declare const ChatContext: React__default.Context<ChatContext | undefined>;
declare function useChatContext(): ChatContext;
interface ChatContextProps {
    labels?: CopilotChatLabels;
    icons?: CopilotChatIcons;
    children?: React__default.ReactNode;
    open: boolean;
    setOpen: (open: boolean) => void;
}
declare const ChatContextProvider: ({ labels, icons, children, open, setOpen, }: ChatContextProps) => react_jsx_runtime.JSX.Element;

export { ChatContext, ChatContextProvider, CopilotChatIcons, CopilotChatLabels, useChatContext };
