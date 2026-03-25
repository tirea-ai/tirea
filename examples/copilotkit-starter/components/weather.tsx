type WeatherCardProps = {
  location: string;
  themeColor: string;
};

export function WeatherCard({ location, themeColor }: WeatherCardProps) {
  return (
    <div
      data-testid="weather-card"
      className="mt-3 overflow-hidden rounded-xl border border-slate-300 bg-white/90 p-0 shadow-[0_10px_24px_rgba(15,23,42,0.08)]"
    >
      <div
        className="flex items-center justify-between border-b border-slate-400 px-3 py-2 text-slate-900"
        style={{
          backgroundColor: themeColor,
        }}
      >
        <strong className="font-bold text-slate-900">Weather</strong>
      </div>
      <div className="grid gap-1 p-3">
        <div className="font-semibold text-slate-900">{location || "Unknown location"}</div>
        <p className="m-0 text-sm text-slate-700">70°F, clear skies (demo card)</p>
      </div>
    </div>
  );
}
