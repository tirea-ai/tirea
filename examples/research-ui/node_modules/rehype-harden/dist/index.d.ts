import type { Root as HastRoot } from "hast";
export declare function harden({ defaultOrigin, allowedLinkPrefixes, allowedImagePrefixes, allowDataImages, allowedProtocols, blockedImageClass, blockedLinkClass, }: {
    defaultOrigin?: string;
    allowedLinkPrefixes?: string[];
    allowedImagePrefixes?: string[];
    allowDataImages?: boolean;
    allowedProtocols?: string[];
    blockedImageClass?: string;
    blockedLinkClass?: string;
}): (tree: HastRoot) => void;
//# sourceMappingURL=index.d.ts.map