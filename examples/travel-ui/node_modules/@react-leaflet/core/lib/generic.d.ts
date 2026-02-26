import type { Control, ControlOptions, FeatureGroup, Layer, Path } from 'leaflet';
import type { PropsWithoutRef } from 'react';
import { type PropsWithChildren } from './component.js';
import type { LeafletContextInterface } from './context.js';
import { type DivOverlay, type DivOverlayLifecycleHook } from './div-overlay.js';
import { type LeafletElement } from './element.js';
import { type LayerProps } from './layer.js';
import { type PathProps } from './path.js';
interface LayerWithChildrenProps extends LayerProps, PropsWithChildren {
}
interface PathWithChildrenProps extends PathProps, PropsWithChildren {
}
export declare function createControlComponent<E extends Control, P extends ControlOptions>(createInstance: (props: PropsWithoutRef<P>) => E): import("react").ForwardRefExoticComponent<PropsWithoutRef<P> & import("react").RefAttributes<E>>;
export declare function createLayerComponent<E extends Layer, P extends LayerWithChildrenProps>(createElement: (props: PropsWithoutRef<P>, context: LeafletContextInterface) => LeafletElement<E>, updateElement?: (instance: E, props: PropsWithoutRef<P>, prevProps: PropsWithoutRef<P>) => void): import("react").ForwardRefExoticComponent<PropsWithoutRef<P> & import("react").RefAttributes<E>>;
export declare function createOverlayComponent<E extends DivOverlay, P extends LayerWithChildrenProps>(createElement: (props: PropsWithoutRef<P>, context: LeafletContextInterface) => LeafletElement<E>, useLifecycle: DivOverlayLifecycleHook<E, PropsWithoutRef<P>>): import("react").ForwardRefExoticComponent<PropsWithoutRef<P> & import("react").RefAttributes<E>>;
export declare function createPathComponent<E extends FeatureGroup | Path, P extends PathWithChildrenProps>(createElement: (props: PropsWithoutRef<P>, context: LeafletContextInterface) => LeafletElement<E>, updateElement?: (instance: E, props: PropsWithoutRef<P>, prevProps: PropsWithoutRef<P>) => void): import("react").ForwardRefExoticComponent<PropsWithoutRef<P> & import("react").RefAttributes<E>>;
export declare function createTileLayerComponent<E extends Layer, P extends LayerProps>(createElement: (props: PropsWithoutRef<P>, context: LeafletContextInterface) => LeafletElement<E>, updateElement?: (instance: E, props: PropsWithoutRef<P>, prevProps: PropsWithoutRef<P>) => void): import("react").ForwardRefExoticComponent<PropsWithoutRef<P> & import("react").RefAttributes<E>>;
export {};
