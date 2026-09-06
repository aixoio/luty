import type { UnlistenFn } from '@tauri-apps/api/event';
import type { SelectedImage } from '$lib/types';
import { IMAGE_EXTENSIONS } from '$lib/types';
import { api, isNativeApp, nativeFileUrl } from '$lib/tauri';

const acceptedExtensions = new Set<string>(IMAGE_EXTENSIONS);
const MAX_PREVIEW_JOBS = 2;

export function fileExtension(path: string): string {
	const filename = path.split(/[\\/]/).pop() ?? '';
	const separator = filename.lastIndexOf('.');
	return separator < 0 ? '' : filename.slice(separator + 1).toLowerCase();
}

export function isAcceptedImage(pathOrName: string): boolean {
	return acceptedExtensions.has(fileExtension(pathOrName));
}

export async function selectNativeImage(path: string): Promise<SelectedImage> {
	if (!isAcceptedImage(path)) {
		throw new Error('Choose a PNG, JPEG, WebP, TIFF, AVIF, GIF, or BMP image.');
	}

	const [info, previewUrl] = await Promise.all([
		api.inspectImage(path),
		api.renderSourcePreview(path).then((result) => nativeFileUrl(result.outputPath))
	]);
	return { kind: 'native', path, info, previewUrl };
}

export function selectNativeImages(paths: string[]): Promise<SelectedImage[]> {
	return mapWithConcurrency(paths, MAX_PREVIEW_JOBS, selectNativeImage);
}

export async function selectBrowserImage(file: File): Promise<SelectedImage> {
	if (!file.type.startsWith('image/') && !isAcceptedImage(file.name)) {
		throw new Error('Choose a supported image file.');
	}
	const bitmap = await createImageBitmap(file, { colorSpaceConversion: 'default' });
	try {
		const scale = Math.min(1, 1600 / Math.max(bitmap.width, bitmap.height));
		const canvas = document.createElement('canvas');
		canvas.width = Math.max(1, Math.round(bitmap.width * scale));
		canvas.height = Math.max(1, Math.round(bitmap.height * scale));
		const context = canvas.getContext('2d', { alpha: true, colorSpace: 'srgb' });
		if (!context) throw new Error('Could not create an SDR image preview.');
		context.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
		const preview = await new Promise<Blob>((resolve, reject) => {
			canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error('Could not encode the SDR image preview.')), 'image/png');
		});
		return { kind: 'browser', file, previewUrl: URL.createObjectURL(preview) };
	} finally {
		bitmap.close();
	}
}

export async function selectBrowserImages(files: File[]): Promise<SelectedImage[]> {
	const images: SelectedImage[] = [];
	try {
		for (const file of files) images.push(await selectBrowserImage(file));
		return images;
	} catch (error) {
		releaseImagePreviews(images);
		throw error;
	}
}

export function releaseImagePreview(image: SelectedImage | null): void {
	if (image?.kind === 'browser') URL.revokeObjectURL(image.previewUrl);
}

export function releaseImagePreviews(images: SelectedImage[]): void {
	for (const image of images) releaseImagePreview(image);
}

async function mapWithConcurrency<T, R>(
	values: T[],
	limit: number,
	transform: (value: T) => Promise<R>
): Promise<R[]> {
	const results = new Array<R>(values.length);
	let nextIndex = 0;
	async function worker() {
		while (nextIndex < values.length) {
			const index = nextIndex++;
			results[index] = await transform(values[index]);
		}
	}
	await Promise.all(Array.from({ length: Math.min(limit, values.length) }, worker));
	return results;
}

/**
 * Listen for operating-system file drops in Tauri. Native drop events retain the
 * absolute path required by the Rust image pipeline. Returns a no-op in a browser.
 */
export async function listenForNativeImageDrops(
	onDrop: (paths: string[]) => void | Promise<void>,
	onHoverChange?: (hovering: boolean) => void
): Promise<UnlistenFn> {
	if (!isNativeApp()) return () => undefined;

	const { getCurrentWebview } = await import('@tauri-apps/api/webview');
	return getCurrentWebview().onDragDropEvent((event) => {
		if (event.payload.type === 'enter' || event.payload.type === 'over') {
			onHoverChange?.(true);
			return;
		}
		if (event.payload.type === 'leave') {
			onHoverChange?.(false);
			return;
		}

		onHoverChange?.(false);
		const paths = event.payload.paths.filter(isAcceptedImage);
		if (paths.length) void onDrop(paths);
	});
}
