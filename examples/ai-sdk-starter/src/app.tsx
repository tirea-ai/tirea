import { Navigate, Route, Routes } from "react-router";
import { PlaygroundPage } from "./pages/playground-page";
import { AdminLayout } from "./components/admin/admin-layout";
import { DashboardPage } from "./pages/admin/dashboard-page";
import { AgentsPage } from "./pages/admin/agents-page";
import { AgentEditorPage } from "./pages/admin/agent-editor-page";
import { ModelsPage } from "./pages/admin/models-page";
import { ProvidersPage } from "./pages/admin/providers-page";
import { AssistantPage } from "./pages/admin/assistant-page";

export function App() {
  return (
    <Routes>
      <Route path="/" element={<PlaygroundPage />} />
      <Route path="/basic" element={<Navigate to="/" replace />} />
      <Route path="/canvas" element={<Navigate to="/" replace />} />
      <Route path="/threads" element={<Navigate to="/" replace />} />

      {/* Admin routes */}
      <Route path="/admin" element={<AdminLayout />}>
        <Route index element={<DashboardPage />} />
        <Route path="agents" element={<AgentsPage />} />
        <Route path="agents/:id" element={<AgentEditorPage />} />
        <Route path="models" element={<ModelsPage />} />
        <Route path="providers" element={<ProvidersPage />} />
        <Route path="assistant" element={<AssistantPage />} />
      </Route>
    </Routes>
  );
}
