import { type RefObject } from 'react';
import type { LeafletContextInterface } from './context.js';
export type LeafletElement<T, C = unknown> = Readonly<{
    instance: T;
    context: LeafletContextInterface;
    container?: C | null;
}>;
export declare function createElementObject<T, C = unknown>(instance: T, context: LeafletContextInterface, container?: C | null): LeafletElement<T, C>;
export type ElementHook<E, P> = (props: P, context: LeafletContextInterface) => RefObject<LeafletElement<E>>;
export declare function createElementHook<E, P, C = unknown>(createElement: (props: P, context: LeafletContextInterface) => LeafletElement<E>, updateElement?: (instance: E, props: P, prevProps: P) => void): (props: P, context: LeafletContextInterface) => ReturnType<ElementHook<E, P>>;
