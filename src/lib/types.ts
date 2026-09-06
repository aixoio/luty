/** File formats accepted by the native image pipeline. */
export const IMAGE_EXTENSIONS = [
	'avif',
	'bmp',
	'gif',
	'jpeg',
	'jpg',
	'png',
	'tif',
	'tiff',
	'webp'
] as const;

export const LUT_EXTENSIONS = ['cube'] as const;

export interface AppSettings {
	lutDirectory: string | null;
}

export interface LutDescriptor {
	path: string;
	name: string;
	title: string | null;
	size: number | null;
	fileSize: number;
	modifiedAtMs: number | null;
	supported: boolean;
	error: string | null;
}

export interface LutCatalog {
	directory: string;
	luts: LutDescriptor[];
}

export interface ImageInfo {
	path: string;
	width: number;
	height: number;
	colorType: string;
	format: string;
}

export interface ProcessImageRequest {
	inputPath: string;
	lutPath: string;
	outputPath: string;
	/** Blend strength from 0 (original) to 1 (full LUT). */
	intensity: number;
}

export interface ProcessImageResult {
	outputPath: string;
	width: number;
	height: number;
	elapsedMs: number;
}

export interface ProcessProgress {
	stage: string;
	percent: number;
}

export type SelectedImage =
	| { kind: 'native'; path: string; info: ImageInfo; previewUrl: string }
	| { kind: 'browser'; file: File; previewUrl: string };

export type WorkPhase = 'idle' | 'loading' | 'processing' | 'complete' | 'error';
export type PreviewPhase = 'idle' | 'rendering' | 'ready' | 'error';

export interface AppState {
	settings: AppSettings;
	catalog: LutCatalog | null;
	selectedImage: SelectedImage | null;
	selectedLutPath: string | null;
	intensity: number;
	phase: WorkPhase;
	result: ProcessImageResult | null;
	previewUrl: string | null;
	previewPhase: PreviewPhase;
	previewError: string | null;
	exportProgress: number;
	exportStage: string;
	error: string | null;
}
