import { derived, get, writable } from 'svelte/store';
import { api, errorMessage } from '$lib/tauri';
import { releaseImagePreview, selectBrowserImage, selectNativeImage } from '$lib/image-input';
import type { AppState, ProcessImageResult } from '$lib/types';

const initialState: AppState = {
	settings: { lutDirectory: null },
	catalog: null,
	selectedImage: null,
	selectedLutPath: null,
	intensity: 1,
	phase: 'idle',
	result: null,
	error: null
};

function createAppStore() {
	const store = writable<AppState>(initialState);
	const { subscribe, update, set } = store;

	async function run<T>(work: () => Promise<T>, apply: (result: T) => Partial<AppState>) {
		update((state) => ({ ...state, phase: 'loading', error: null }));
		try {
			const result = await work();
			update((state) => ({ ...state, ...apply(result), phase: 'idle' }));
			return result;
		} catch (error) {
			update((state) => ({ ...state, phase: 'error', error: errorMessage(error) }));
			throw error;
		}
	}

	return {
		subscribe,
		reset() {
			releaseImagePreview(get(store).selectedImage);
			set(initialState);
		},
		clearError() {
			update((state) => ({ ...state, error: null, phase: state.phase === 'error' ? 'idle' : state.phase }));
		},
		async hydrate() {
			const settings = await run(api.getSettings, (value) => ({ settings: value }));
			if (settings.lutDirectory) {
				await run(api.listLuts, (catalog) => ({ catalog }));
			}
		},
		async setLutDirectory(path: string) {
			return run(() => api.setLutDirectory(path), (catalog) => ({
				catalog,
				settings: { lutDirectory: catalog.directory },
				selectedLutPath: null
			}));
		},
		async chooseLutDirectory() {
			const path = await api.chooseLutDirectory();
			if (!path) return null;
			return this.setLutDirectory(path);
		},
		async refreshLuts() {
			return run(api.listLuts, (catalog) => ({ catalog }));
		},
		async selectNativeImage(path: string) {
			const previous = get(store).selectedImage;
			const image = await run(() => selectNativeImage(path), (selectedImage) => ({
				selectedImage,
				result: null
			}));
			releaseImagePreview(previous);
			return image;
		},
		async chooseNativeImage() {
			const path = await api.chooseImage();
			if (!path) return null;
			return this.selectNativeImage(path);
		},
		selectBrowserImage(file: File) {
			const previous = get(store).selectedImage;
			const selectedImage = selectBrowserImage(file);
			update((state) => ({ ...state, selectedImage, result: null, error: null }));
			releaseImagePreview(previous);
		},
		selectLut(path: string | null) {
			update((state) => ({ ...state, selectedLutPath: path, result: null }));
		},
		setIntensity(value: number) {
			update((state) => ({ ...state, intensity: Math.min(1, Math.max(0, value)), result: null }));
		},
		async process(outputPath: string): Promise<ProcessImageResult> {
			const state = get(store);
			if (state.selectedImage?.kind !== 'native') {
				throw new Error('Choose an image in the desktop app before exporting.');
			}
			if (!state.selectedLutPath) throw new Error('Choose a LUT before exporting.');

			update((current) => ({ ...current, phase: 'processing', error: null }));
			try {
				const result = await api.processImage({
					inputPath: state.selectedImage.path,
					lutPath: state.selectedLutPath,
					outputPath,
					intensity: state.intensity
				});
				update((current) => ({ ...current, result, phase: 'complete' }));
				return result;
			} catch (error) {
				update((current) => ({ ...current, phase: 'error', error: errorMessage(error) }));
				throw error;
			}
		},
		async chooseAndProcess(suggestedName?: string): Promise<ProcessImageResult | null> {
			const outputPath = await api.chooseOutputPath(suggestedName);
			if (!outputPath) return null;
			return this.process(outputPath);
		}
	};
}

export const appStore = createAppStore();

export const supportedLuts = derived(appStore, ($state) =>
	($state.catalog?.luts ?? []).filter((lut) => lut.supported)
);

export const canProcess = derived(
	appStore,
	($state) => $state.selectedImage?.kind === 'native' && Boolean($state.selectedLutPath)
);
