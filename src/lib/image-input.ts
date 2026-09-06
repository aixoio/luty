import type { UnlistenFn } from '@tauri-apps/api/event';
import type { SelectedImage } from '$lib/types';
import { IMAGE_EXTENSIONS } from '$lib/types';
import { api, isNativeApp, nativeFileUrl } from '$lib/tauri';

const acceptedExtensions = new Set<string>(IMAGE_EXTENSIONS);

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

	const [info, previewUrl] = await Promise.all([api.inspectImage(path), nativeFileUrl(path)]);
	return { kind: 'native', path, info, previewUrl };
}

export function selectNativeImages(paths: string[]): Promise<SelectedImage[]> {
	return Promise.all(paths.map(selectNativeImage));
}

export function selectBrowserImage(file: File): SelectedImage {
	if (!file.type.startsWith('image/') && !isAcceptedImage(file.name)) {
		throw new Error('Choose a supported image file.');
	}
	return { kind: 'browser', file, previewUrl: URL.createObjectURL(file) };
}

export function selectBrowserImages(files: File[]): SelectedImage[] {
	return files.map(selectBrowserImage);
}

export function releaseImagePreview(image: SelectedImage | null): void {
	if (image?.kind === 'browser') URL.revokeObjectURL(image.previewUrl);
}

export function releaseImagePreviews(images: SelectedImage[]): void {
	for (const image of images) releaseImagePreview(image);
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
