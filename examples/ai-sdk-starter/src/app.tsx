import { Navigate, Route, Routes } from "react-router";
import { PlaygroundPage } from "./pages/playground-page";

export function App() {
  return (
    <Routes>
      <Route path="/" element={<PlaygroundPage />} />
      <Route path="/basic" element={<Navigate to="/" replace />} />
      <Route path="/canvas" element={<Navigate to="/" replace />} />
      <Route path="/threads" element={<Navigate to="/" replace />} />
    </Routes>
  );
}
