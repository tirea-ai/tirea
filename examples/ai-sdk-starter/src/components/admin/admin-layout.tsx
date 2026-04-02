import { Link, Outlet, useLocation } from "react-router";

const navItems = [
  { path: "/admin", label: "Dashboard" },
  { path: "/admin/agents", label: "Agents" },
  { path: "/admin/models", label: "Models" },
  { path: "/admin/providers", label: "Providers" },
];

export function AdminLayout() {
  const location = useLocation();

  return (
    <div className="flex h-screen">
      {/* Sidebar */}
      <nav className="w-56 bg-gray-900 text-gray-300 flex flex-col">
        <div className="p-4 border-b border-gray-700">
          <Link to="/" className="text-sm text-gray-500 hover:text-gray-300">
            &larr; Playground
          </Link>
          <h1 className="text-lg font-semibold text-white mt-1">Admin</h1>
        </div>
        <ul className="flex-1 py-2">
          {navItems.map((item) => (
            <li key={item.path}>
              <Link
                to={item.path}
                className={`block px-4 py-2 text-sm hover:bg-gray-800 ${
                  location.pathname === item.path
                    ? "bg-gray-800 text-white"
                    : ""
                }`}
              >
                {item.label}
              </Link>
            </li>
          ))}
        </ul>
        <div className="p-4 border-t border-gray-700">
          <Link
            to="/admin/assistant"
            className="block px-3 py-2 text-sm bg-blue-600 hover:bg-blue-500 text-white rounded text-center"
          >
            AI Config Assistant
          </Link>
        </div>
      </nav>

      {/* Main content */}
      <main className="flex-1 overflow-auto bg-gray-50">
        <Outlet />
      </main>
    </div>
  );
}
