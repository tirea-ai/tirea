import {
  ActivityIcon,
  CloseIcon,
  CopyIcon,
  HeaderCloseIcon,
  MicrophoneIcon,
  OpenIcon,
  RegenerateIcon,
  SendIcon,
  SpinnerIcon,
  StopIcon,
  ThumbsDownIcon,
  ThumbsUpIcon,
  UploadIcon
} from "./chunk-XWG3L6QC.mjs";
import {
  __spreadValues
} from "./chunk-MRXNTQOX.mjs";

// src/components/chat/ChatContext.tsx
import React, { useMemo } from "react";
import { jsx } from "react/jsx-runtime";
var ChatContext = React.createContext(void 0);
function useChatContext() {
  const context = React.useContext(ChatContext);
  if (context === void 0) {
    throw new Error(
      "Context not found. Did you forget to wrap your app in a <ChatContextProvider> component?"
    );
  }
  return context;
}
var ChatContextProvider = ({
  // temperature,
  // instructions,
  // maxFeedback,
  labels,
  icons,
  children,
  open,
  setOpen
}) => {
  const memoizedLabels = useMemo(
    () => __spreadValues(__spreadValues({}, {
      initial: "",
      title: "CopilotKit",
      placeholder: "Type a message...",
      error: "\u274C An error occurred. Please try again.",
      stopGenerating: "Stop generating",
      regenerateResponse: "Regenerate response",
      copyToClipboard: "Copy to clipboard",
      thumbsUp: "Thumbs up",
      thumbsDown: "Thumbs down",
      copied: "Copied!"
    }), labels),
    [labels]
  );
  const memoizedIcons = useMemo(
    () => __spreadValues(__spreadValues({}, {
      openIcon: OpenIcon,
      closeIcon: CloseIcon,
      headerCloseIcon: HeaderCloseIcon,
      sendIcon: SendIcon,
      activityIcon: ActivityIcon,
      spinnerIcon: SpinnerIcon,
      stopIcon: StopIcon,
      regenerateIcon: RegenerateIcon,
      pushToTalkIcon: MicrophoneIcon,
      copyIcon: CopyIcon,
      thumbsUpIcon: ThumbsUpIcon,
      thumbsDownIcon: ThumbsDownIcon,
      uploadIcon: UploadIcon
    }), icons),
    [icons]
  );
  const context = useMemo(
    () => ({
      labels: memoizedLabels,
      icons: memoizedIcons,
      open,
      setOpen
    }),
    [memoizedLabels, memoizedIcons, open, setOpen]
  );
  return /* @__PURE__ */ jsx(ChatContext.Provider, { value: context, children });
};

export {
  ChatContext,
  useChatContext,
  ChatContextProvider
};
//# sourceMappingURL=chunk-IEMQ2SQW.mjs.map