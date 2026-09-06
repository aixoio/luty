import type {
	AppSettings,
	ImageInfo,
	LutCatalog,
	ProcessImageRequest,
	ProcessImageResult
} from '$lib/types';

/** A predictable error shape for both rejected Tauri commands and browser preview. */
export class LutyError extends Error {
	constructor(
		message: string,
		public readonly cause?: unknown
	) {
		super(message);
		this.name = 'LutyError';
	}
}

export function isNativeApp(): boolean {
	return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
	if (!isNativeApp()) {
		throw new LutyError('This action is available in the Luty desktop app.');
	}

	try {
		const { invoke } = await import('@tauri-apps/api/core');
		return await invoke<T>(command, args);
	} catch (error) {
		if (error instanceof LutyError) throw error;
		const message = typeof error === 'string' ? error : 'The desktop command did not complete.';
		throw new LutyError(message, error);
	}
}

export const api = {
	getSettings: () => call<AppSettings>('get_settings'),
	listLuts: () => call<LutCatalog>('list_luts'),
	setLutDirectory: (path: string) => call<LutCatalog>('set_lut_directory', { path }),
	chooseImage: () => call<string | null>('choose_image'),
	chooseLutDirectory: () => call<string | null>('choose_lut_directory'),
	chooseOutputPath: (suggestedName?: string) =>
		call<string | null>('choose_output_path', { suggestedName: suggestedName ?? null }),
	inspectImage: (inputPath: string) => call<ImageInfo>('inspect_image', { inputPath }),
	processImage: (request: ProcessImageRequest) =>
		call<ProcessImageResult>('process_image', { request })
};

export async function nativeFileUrl(path: string): Promise<string> {
	if (!isNativeApp()) return '';
	const { convertFileSrc } = await import('@tauri-apps/api/core');
	return convertFileSrc(path);
}

export function errorMessage(error: unknown): string {
	if (error instanceof Error) return error.message;
	if (typeof error === 'string') return error;
	return 'Something went wrong.';
}
