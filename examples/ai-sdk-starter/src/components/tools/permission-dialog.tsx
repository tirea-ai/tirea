type PermissionDialogProps = {
  requestedToolName: string;
  approvalId: string;
  onApprove: (id: string) => void;
  onDeny: (id: string) => void;
  themeMode?: "light" | "dark";
};

export function PermissionDialog({
  requestedToolName,
  approvalId,
  onApprove,
  onDeny,
  themeMode = "light",
}: PermissionDialogProps) {
  const isDark = themeMode === "dark";
  return (
    <div
      data-testid="permission-dialog"
      className={
        isDark
          ? "mt-2 rounded-lg border border-slate-700 bg-slate-900 p-3"
          : "mt-2 rounded-lg border border-slate-200 bg-white p-3"
      }
    >
      <div className={isDark ? "mb-2 text-sm text-slate-200" : "mb-2 text-sm text-slate-800"}>
        Approve tool &apos;{requestedToolName}&apos; execution?
      </div>
      <div className="flex gap-2">
        <button
          data-testid="permission-allow"
          onClick={() => onApprove(approvalId)}
          className="rounded-lg border border-green-300 bg-green-50 px-3 py-1.5 text-sm font-semibold text-green-800 transition hover:bg-green-100"
        >
          Allow
        </button>
        <button
          data-testid="permission-deny"
          onClick={() => onDeny(approvalId)}
          className="rounded-lg border border-red-300 bg-red-50 px-3 py-1.5 text-sm font-semibold text-red-800 transition hover:bg-red-100"
        >
          Deny
        </button>
      </div>
    </div>
  );
}
