export interface Place {
  id: string;
  name: string;
  address: string;
  lat: number;
  lng: number;
  description: string;
  category: string;
}

export interface Trip {
  id: string;
  name: string;
  destination: string;
  places: Place[];
}

export interface SearchProgress {
  query: string;
  status: string;
  results_count: number;
}

export interface TravelState {
  selected_trip_id: string | null;
  trips: Trip[];
  search_progress: SearchProgress[];
}
