import { createContext } from 'svelte';
import { api, errorMessage, nativeFileUrl } from '$lib/tauri';
import { releaseImagePreview, selectBrowserImage, selectNativeImage } from '$lib/image-input';
import type { AppState, ProcessImageResult } from '$lib/types';

function initialState(): AppState {
	return {
		settings: { lutDirectory: null },
		catalog: null,
		selectedImage: null,
		selectedLutPath: null,
		intensity: 1,
		phase: 'idle',
		result: null,
		previewUrl: null,
		previewPhase: 'idle',
		previewError: null,
		exportProgress: 0,
		exportStage: '',
		error: null
	};
}

export class AppContext {
	state = $state<AppState>(initialState());
	supportedLuts = $derived((this.state.catalog?.luts ?? []).filter((lut) => lut.supported));
	canProcess = $derived(
		this.state.selectedImage?.kind === 'native' && Boolean(this.state.selectedLutPath)
	);
	private previewTimer: ReturnType<typeof setTimeout> | undefined;
	private previewGeneration = 0;

	private async run<T>(work: () => Promise<T>, apply: (result: T) => Partial<AppState>) {
		this.state.phase = 'loading';
		this.state.error = null;
		try {
			const result = await work();
			Object.assign(this.state, apply(result));
			this.state.phase = 'idle';
			return result;
		} catch (error) {
			this.state.phase = 'error';
			this.state.error = errorMessage(error);
			throw error;
		}
	}

	reset() {
		if (this.previewTimer) clearTimeout(this.previewTimer);
		this.previewGeneration += 1;
		releaseImagePreview(this.state.selectedImage);
		Object.assign(this.state, initialState());
	}

	clearError() {
		this.state.error = null;
		if (this.state.phase === 'error') this.state.phase = 'idle';
	}

	async hydrate() {
		const settings = await this.run(api.getSettings, (value) => ({ settings: value }));
		if (settings.lutDirectory) {
			await this.run(api.listLuts, (catalog) => ({ catalog }));
		}
	}

	async setLutDirectory(path: string) {
		this.clearPreview();
		return this.run(() => api.setLutDirectory(path), (catalog) => ({
			catalog,
			settings: { lutDirectory: catalog.directory },
			selectedLutPath: null
		}));
	}

	async chooseLutDirectory() {
		const path = await api.chooseLutDirectory();
		if (!path) return null;
		return this.setLutDirectory(path);
	}

	async refreshLuts() {
		return this.run(api.listLuts, (catalog) => ({ catalog }));
	}

	async selectNativeImage(path: string) {
		const previous = this.state.selectedImage;
		this.clearPreview();
		const image = await this.run(() => selectNativeImage(path), (selectedImage) => ({
			selectedImage,
			result: null
		}));
		releaseImagePreview(previous);
		this.schedulePreview(0);
		return image;
	}

	async chooseNativeImage() {
		const path = await api.chooseImage();
		if (!path) return null;
		return this.selectNativeImage(path);
	}

	selectBrowserImage(file: File) {
		this.clearPreview();
		const previous = this.state.selectedImage;
		this.state.selectedImage = selectBrowserImage(file);
		this.state.result = null;
		this.state.error = null;
		releaseImagePreview(previous);
	}

	selectLut(path: string | null) {
		this.state.selectedLutPath = path;
		this.state.result = null;
		this.clearPreview();
		this.schedulePreview(0);
	}

	setIntensity(value: number) {
		this.state.intensity = Math.min(1, Math.max(0, value));
		this.state.result = null;
		this.schedulePreview();
	}

	async process(outputPath: string): Promise<ProcessImageResult> {
		const image = this.state.selectedImage;
		if (image?.kind !== 'native') {
			throw new Error('Choose an image in the desktop app before exporting.');
		}
		if (!this.state.selectedLutPath) throw new Error('Choose a LUT before exporting.');

		this.state.phase = 'processing';
		this.state.error = null;
		this.state.exportProgress = 5;
		this.state.exportStage = 'Preparing image';
		try {
			const result = await api.processImage({
				inputPath: image.path,
				lutPath: this.state.selectedLutPath,
				outputPath,
				intensity: this.state.intensity
			}, (progress) => {
				this.state.exportProgress = progress.percent;
				this.state.exportStage = progress.stage;
			});
			this.state.result = result;
			this.state.exportProgress = 100;
			this.state.exportStage = 'Export complete';
			this.state.phase = 'complete';
			return result;
		} catch (error) {
			this.state.phase = 'error';
			this.state.error = errorMessage(error);
			throw error;
		}
	}

	async chooseAndProcess(suggestedName?: string): Promise<ProcessImageResult | null> {
		const outputPath = await api.chooseOutputPath(suggestedName);
		if (!outputPath) return null;
		return this.process(outputPath);
	}

	private clearPreview() {
		if (this.previewTimer) clearTimeout(this.previewTimer);
		this.previewTimer = undefined;
		this.previewGeneration += 1;
		this.state.previewUrl = null;
		this.state.previewPhase = 'idle';
		this.state.previewError = null;
	}

	private schedulePreview(delay = 140) {
		if (this.previewTimer) clearTimeout(this.previewTimer);
		const image = this.state.selectedImage;
		if (image?.kind !== 'native' || !this.state.selectedLutPath) return;
		this.previewTimer = setTimeout(() => void this.renderPreview(), delay);
	}

	private async renderPreview() {
		const image = this.state.selectedImage;
		const lutPath = this.state.selectedLutPath;
		if (image?.kind !== 'native' || !lutPath) return;
		const generation = ++this.previewGeneration;
		this.state.previewPhase = 'rendering';
		this.state.previewError = null;
		try {
			const result = await api.renderPreview(image.path, lutPath, this.state.intensity);
			const url = await nativeFileUrl(result.outputPath);
			if (generation !== this.previewGeneration) return;
			this.state.previewUrl = url;
			this.state.previewPhase = 'ready';
		} catch (error) {
			if (generation !== this.previewGeneration) return;
			this.state.previewPhase = 'error';
			this.state.previewError = errorMessage(error);
		}
	}
}

const [getAppContext, setAppContext] = createContext<AppContext>();

export { getAppContext };

export function provideAppContext() {
	return setAppContext(new AppContext());
}
