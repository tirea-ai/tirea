import { type PathProps } from '@react-leaflet/core';
import { type LatLngExpression, Polygon as LeafletPolygon, type PolylineOptions } from 'leaflet';
import type { ReactNode } from 'react';
export interface PolygonProps extends PolylineOptions, PathProps {
    children?: ReactNode;
    positions: LatLngExpression[] | LatLngExpression[][] | LatLngExpression[][][];
}
export declare const Polygon: import("react").ForwardRefExoticComponent<PolygonProps & import("react").RefAttributes<LeafletPolygon<any>>>;
