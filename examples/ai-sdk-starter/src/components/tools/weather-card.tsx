type WeatherCardProps = {
  location: string;
};

export function WeatherCard({ location }: WeatherCardProps) {
  return (
    <div
      data-testid="weather-card"
      className="my-2 overflow-hidden rounded-xl border border-slate-300 bg-white/90 shadow-sm"
    >
      <div className="flex items-center justify-between border-b border-slate-200 bg-cyan-50 px-3 py-2">
        <strong className="font-bold text-slate-900">Weather</strong>
      </div>
      <div className="grid gap-1 p-3">
        <div className="font-semibold text-slate-900">
          {location || "Unknown location"}
        </div>
        <p className="m-0 text-sm text-slate-700">
          70F, clear skies (demo card)
        </p>
      </div>
    </div>
  );
}
