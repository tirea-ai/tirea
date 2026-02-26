import type { Control, Layer, LayerGroup, Map as LeafletMap } from 'leaflet';
export declare const CONTEXT_VERSION = 1;
export type ControlledLayer = {
    addLayer(layer: Layer): void;
    removeLayer(layer: Layer): void;
};
export type LeafletContextInterface = Readonly<{
    __version: number;
    map: LeafletMap;
    layerContainer?: ControlledLayer | LayerGroup;
    layersControl?: Control.Layers;
    overlayContainer?: Layer;
    pane?: string;
}>;
export declare function createLeafletContext(map: LeafletMap): LeafletContextInterface;
export declare function extendContext(source: LeafletContextInterface, extra: Partial<LeafletContextInterface>): LeafletContextInterface;
export declare const LeafletContext: import("react").Context<Readonly<{
    __version: number;
    map: LeafletMap;
    layerContainer?: ControlledLayer | LayerGroup;
    layersControl?: Control.Layers;
    overlayContainer?: Layer;
    pane?: string;
}> | null>;
export declare function useLeafletContext(): LeafletContextInterface;
