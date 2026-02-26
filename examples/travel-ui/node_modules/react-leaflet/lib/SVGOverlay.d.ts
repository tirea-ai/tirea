import { type MediaOverlayProps } from '@react-leaflet/core';
import { SVGOverlay as LeafletSVGOverlay } from 'leaflet';
import { type ReactNode } from 'react';
export interface SVGOverlayProps extends MediaOverlayProps {
    attributes?: Record<string, string>;
    children?: ReactNode;
}
export declare const useSVGOverlayElement: (props: SVGOverlayProps, context: Readonly<{
    __version: number;
    map: import("leaflet").Map;
    layerContainer?: import("@react-leaflet/core/lib/context").ControlledLayer | import("leaflet").LayerGroup;
    layersControl?: import("leaflet").Control.Layers;
    overlayContainer?: import("leaflet").Layer;
    pane?: string;
}>) => import("react").RefObject<Readonly<{
    instance: LeafletSVGOverlay;
    context: Readonly<{
        __version: number;
        map: import("leaflet").Map;
        layerContainer?: import("@react-leaflet/core/lib/context").ControlledLayer | import("leaflet").LayerGroup;
        layersControl?: import("leaflet").Control.Layers;
        overlayContainer?: import("leaflet").Layer;
        pane?: string;
    }>;
    container?: unknown;
}>>;
export declare const useSVGOverlay: (props: SVGOverlayProps) => import("react").RefObject<Readonly<{
    instance: LeafletSVGOverlay;
    context: Readonly<{
        __version: number;
        map: import("leaflet").Map;
        layerContainer?: import("@react-leaflet/core/lib/context").ControlledLayer | import("leaflet").LayerGroup;
        layersControl?: import("leaflet").Control.Layers;
        overlayContainer?: import("leaflet").Layer;
        pane?: string;
    }>;
    container?: unknown;
}>>;
export declare const SVGOverlay: import("react").ForwardRefExoticComponent<SVGOverlayProps & import("react").RefAttributes<LeafletSVGOverlay>>;
