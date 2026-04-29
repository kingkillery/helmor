import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const apiMocks = vi.hoisted(() => ({
	getCliStatus: vi.fn(),
	installCli: vi.fn(),
}));

vi.mock("@/lib/api", async (importOriginal) => {
	const actual = await importOriginal<typeof import("@/lib/api")>();
	return {
		...actual,
		getCliStatus: apiMocks.getCliStatus,
		installCli: apiMocks.installCli,
	};
});

import { CliInstallPanel } from "./cli-install";

describe("CliInstallPanel", () => {
	beforeEach(() => {
		apiMocks.getCliStatus.mockReset();
		apiMocks.installCli.mockReset();
	});

	afterEach(() => {
		cleanup();
		vi.clearAllMocks();
	});

	it("renders the managed install state", async () => {
		apiMocks.getCliStatus.mockResolvedValue({
			installed: true,
			installPath: "/usr/local/bin/helmor-dev",
			buildMode: "development",
			installState: "managed",
			platform: "macos",
		});

		render(<CliInstallPanel />);

		await waitFor(() => {
			expect(screen.getByText(/Installed at/)).toBeInTheDocument();
		});
		expect(screen.getByText("helmor-dev")).toBeInTheDocument();
		expect(screen.getByText("/usr/local/bin/helmor-dev")).toBeInTheDocument();
		expect(
			screen.getByRole("button", { name: "Reinstall" }),
		).toBeInTheDocument();
	});

	it("renders the stale install state and allows reinstall", async () => {
		const user = userEvent.setup();
		apiMocks.getCliStatus.mockResolvedValue({
			installed: true,
			installPath: "/usr/local/bin/helmor",
			buildMode: "release",
			installState: "stale",
			platform: "macos",
		});
		apiMocks.installCli.mockResolvedValue({
			installed: true,
			installPath: "/usr/local/bin/helmor",
			buildMode: "release",
			installState: "managed",
			platform: "macos",
		});

		render(<CliInstallPanel />);

		await waitFor(() => {
			expect(
				screen.getByText(/is not managed by this app/i),
			).toBeInTheDocument();
		});

		await user.click(screen.getByRole("button", { name: "Reinstall" }));

		await waitFor(() => {
			expect(apiMocks.installCli).toHaveBeenCalledTimes(1);
		});
		await waitFor(() => {
			expect(screen.getByText(/Installed at/)).toBeInTheDocument();
		});
	});

	it("renders the Windows install hint", async () => {
		apiMocks.getCliStatus.mockResolvedValue({
			installed: false,
			installPath: null,
			buildMode: "release",
			installState: "missing",
			platform: "windows",
		});

		render(<CliInstallPanel />);

		await waitFor(() => {
			expect(
				screen.getByText(/Add that folder to your PATH once on Windows/i),
			).toBeInTheDocument();
		});
		expect(
			screen.getByRole("button", { name: "Install to ~/.helmor/bin" }),
		).toBeInTheDocument();
	});
});
