import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import "@testing-library/jest-dom/vitest";
import { PermissionDialog } from "./permission-dialog";
import { AskUserDialog } from "./ask-user-dialog";

describe("tool dialogs theme", () => {
  it("applies dark theme styles for permission dialog", () => {
    const onApprove = vi.fn();
    const onDeny = vi.fn();
    render(
      <PermissionDialog
        requestedToolName="append_note"
        approvalId="a1"
        onApprove={onApprove}
        onDeny={onDeny}
        themeMode="dark"
      />,
    );

    const dialog = screen.getByTestId("permission-dialog");
    expect(dialog).toHaveClass("bg-slate-900");
    expect(dialog).toHaveClass("border-slate-700");

    fireEvent.click(screen.getByTestId("permission-allow"));
    expect(onApprove).toHaveBeenCalledWith("a1");
  });

  it("applies dark theme styles for ask dialog", () => {
    render(
      <AskUserDialog
        toolCallId="c1"
        prompt="Question?"
        value="answer"
        onChange={() => {}}
        onSubmit={() => {}}
        disabled={false}
        themeMode="dark"
      />,
    );

    const dialog = screen.getByTestId("ask-dialog");
    const input = screen.getByTestId("ask-question-input");
    expect(dialog).toHaveClass("bg-slate-900");
    expect(input).toHaveClass("bg-slate-950");
  });
});

