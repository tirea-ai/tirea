import React, { type PropsWithoutRef, type RefObject, type ReactNode } from 'react';
import type { DivOverlay, DivOverlayHook } from './div-overlay.js';
import type { LeafletElement } from './element.js';
type ElementHook<E, P> = (props: P) => RefObject<LeafletElement<E>>;
export type PropsWithChildren = PropsWithoutRef<{
    children?: ReactNode;
}>;
export declare function createContainerComponent<E, P extends PropsWithChildren>(useElement: ElementHook<E, PropsWithoutRef<P>>): React.ForwardRefExoticComponent<React.PropsWithoutRef<P> & React.RefAttributes<E>>;
export declare function createDivOverlayComponent<E extends DivOverlay, P extends PropsWithChildren>(useElement: ReturnType<DivOverlayHook<E, PropsWithoutRef<P>>>): React.ForwardRefExoticComponent<React.PropsWithoutRef<P> & React.RefAttributes<E>>;
export declare function createLeafComponent<E, P>(useElement: ElementHook<E, PropsWithoutRef<P>>): React.ForwardRefExoticComponent<React.PropsWithoutRef<P> & React.RefAttributes<E>>;
export {};
