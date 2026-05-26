/**
 * Remote Terminal Store
 *
 * Manages the state of the remote terminal HTTP + WebSocket server.
 * Provides reactive state for UI components (StatusBar, AppPreferences).
 */

import { create } from "zustand";
import { remoteApi, type RemoteServerInfo } from "@/lib/remote-api";

interface RemoteState {
	/** Whether the remote server is currently running */
	isRunning: boolean;
	/** Port the server is listening on */
	port: number | null;
	/** Authentication token */
	token: string | null;
	/** Full URL with token for easy sharing */
	url: string | null;
	/** Whether an operation is in progress */
	isLoading: boolean;
	/** Last error message */
	error: string | null;

	// Actions
	start: (port?: number, bindLan?: boolean) => Promise<void>;
	stop: () => Promise<void>;
	refreshStatus: () => Promise<void>;
	clearError: () => void;
}

export const useRemoteStore = create<RemoteState>((set) => ({
	isRunning: false,
	port: null,
	token: null,
	url: null,
	isLoading: false,
	error: null,

	start: async (port?: number, bindLan?: boolean) => {
		set({ isLoading: true, error: null });
		try {
			const result = await remoteApi.start(port, bindLan);
			if (result.success && result.data) {
				const info: RemoteServerInfo = result.data;
				set({
					isRunning: true,
					port: info.port,
					token: info.token,
					url: info.url,
					isLoading: false,
				});
			} else {
				set({
					isLoading: false,
					error: result.error ?? "Failed to start remote server",
				});
			}
		} catch (err) {
			set({
				isLoading: false,
				error: err instanceof Error ? err.message : String(err),
			});
		}
	},

	stop: async () => {
		set({ isLoading: true, error: null });
		try {
			const result = await remoteApi.stop();
			if (result.success) {
				set({
					isRunning: false,
					port: null,
					token: null,
					url: null,
					isLoading: false,
				});
			} else {
				set({
					isLoading: false,
					error: result.error ?? "Failed to stop remote server",
				});
			}
		} catch (err) {
			set({
				isLoading: false,
				error: err instanceof Error ? err.message : String(err),
			});
		}
	},

	refreshStatus: async () => {
		try {
			const result = await remoteApi.status();
			if (result.success && result.data) {
				set({
					isRunning: result.data.running,
					port: result.data.port ?? null,
					token: result.data.token ?? null,
					url: result.data.url ?? null,
				});
			}
		} catch {
			// Silently fail — status check is non-critical
		}
	},

	clearError: () => set({ error: null }),
}));

// Selectors for common use cases
export const useRemoteIsRunning = (): boolean =>
	useRemoteStore((s) => s.isRunning);
export const useRemoteUrl = (): string | null => useRemoteStore((s) => s.url);
export const useRemotePort = (): number | null => useRemoteStore((s) => s.port);
