import { Control, type Layer } from 'leaflet';
import React, { type ForwardRefExoticComponent, type FunctionComponent, type ReactNode, type RefAttributes } from 'react';
export interface LayersControlProps extends Control.LayersOptions {
    children?: ReactNode;
}
export declare const useLayersControlElement: (props: LayersControlProps, context: Readonly<{
    __version: number;
    map: import("leaflet").Map;
    layerContainer?: import("@react-leaflet/core/lib/context").ControlledLayer | import("leaflet").LayerGroup;
    layersControl?: Control.Layers;
    overlayContainer?: Layer;
    pane?: string;
}>) => React.RefObject<Readonly<{
    instance: Control.Layers;
    context: Readonly<{
        __version: number;
        map: import("leaflet").Map;
        layerContainer?: import("@react-leaflet/core/lib/context").ControlledLayer | import("leaflet").LayerGroup;
        layersControl?: Control.Layers;
        overlayContainer?: Layer;
        pane?: string;
    }>;
    container?: unknown;
}>>;
export declare const useLayersControl: (props: LayersControlProps) => React.RefObject<Readonly<{
    instance: Control.Layers;
    context: Readonly<{
        __version: number;
        map: import("leaflet").Map;
        layerContainer?: import("@react-leaflet/core/lib/context").ControlledLayer | import("leaflet").LayerGroup;
        layersControl?: Control.Layers;
        overlayContainer?: Layer;
        pane?: string;
    }>;
    container?: unknown;
}>>;
export interface ControlledLayerProps {
    checked?: boolean;
    children: ReactNode;
    name: string;
}
export declare const LayersControl: ForwardRefExoticComponent<LayersControlProps & RefAttributes<Control.Layers>> & {
    BaseLayer: FunctionComponent<ControlledLayerProps>;
    Overlay: FunctionComponent<ControlledLayerProps>;
};
type AddLayerFunc = (layersControl: Control.Layers, layer: Layer, name: string) => void;
export declare function createControlledLayer(addLayerToControl: AddLayerFunc): (props: ControlledLayerProps) => React.JSX.Element | null;
export {};
