import type {
	AppSettings,
	ImageInfo,
	LutCatalog,
	ProcessProgress,
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
	chooseImages: () => call<string[]>('choose_images'),
	chooseLutDirectory: () => call<string | null>('choose_lut_directory'),
	chooseOutputPath: (suggestedName?: string) =>
		call<string | null>('choose_output_path', { suggestedName: suggestedName ?? null }),
	chooseOutputDirectory: () => call<string | null>('choose_output_directory'),
	availableOutputPaths: (directory: string, names: string[]) =>
		call<string[]>('available_output_paths', { directory, names }),
	inspectImage: (inputPath: string) => call<ImageInfo>('inspect_image', { inputPath }),
	processImage: async (
		request: ProcessImageRequest,
		onProgress: (progress: ProcessProgress) => void
	) => {
		const { Channel } = await import('@tauri-apps/api/core');
		const progress = new Channel<ProcessProgress>();
		progress.onmessage = onProgress;
		return call<ProcessImageResult>('process_image', { request, progress });
	},
	renderPreview: (inputPath: string, lutPath: string, intensity: number) =>
		call<ProcessImageResult>('render_preview', { inputPath, lutPath, intensity })
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
