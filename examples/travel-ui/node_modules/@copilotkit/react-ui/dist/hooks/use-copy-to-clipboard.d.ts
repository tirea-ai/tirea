interface useCopyToClipboardProps {
    timeout?: number;
}
declare function useCopyToClipboard({ timeout }: useCopyToClipboardProps): {
    isCopied: Boolean;
    copyToClipboard: (value: string) => void;
};

export { useCopyToClipboard, useCopyToClipboardProps };
