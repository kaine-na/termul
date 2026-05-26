/**
 * Remote Terminal API
 *
 * Wraps Tauri commands for remote terminal server control.
 * Follows the IpcResult<T> pattern used throughout the app.
 */

import { invoke } from "@tauri-apps/api/core";

export interface RemoteServerInfo {
	port: number;
	token: string;
	url: string;
	bindLan: boolean;
	warning?: string;
}

export interface RemoteServerStatus {
	running: boolean;
	port?: number;
	token?: string;
	url?: string;
}

interface IpcResult<T> {
	success: boolean;
	data?: T;
	error?: string;
	code?: string;
}

/**
 * Start the remote terminal HTTP + WebSocket server.
 */
export async function remoteStart(
	port?: number,
	bindLan?: boolean,
): Promise<IpcResult<RemoteServerInfo>> {
	return await invoke<IpcResult<RemoteServerInfo>>("remote_start", {
		port: port ?? null,
		bindLan: bindLan ?? null,
	});
}

/**
 * Stop the remote terminal server.
 */
export async function remoteStop(): Promise<IpcResult<void>> {
	return await invoke<IpcResult<void>>("remote_stop");
}

/**
 * Get current status of the remote terminal server.
 */
export async function remoteStatus(): Promise<IpcResult<RemoteServerStatus>> {
	return await invoke<IpcResult<RemoteServerStatus>>("remote_status");
}

export const remoteApi = {
	start: remoteStart,
	stop: remoteStop,
	status: remoteStatus,
};
