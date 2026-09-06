import { createContext } from 'svelte';
import { api, errorMessage, nativeFileUrl } from '$lib/tauri';
import {
	releaseImagePreview,
	releaseImagePreviews,
	selectBrowserImages,
	selectNativeImages
} from '$lib/image-input';
import type { AppState, ProcessImageResult, SelectedImage } from '$lib/types';

function initialState(): AppState {
	return {
		settings: { lutDirectory: null },
		catalog: null,
		images: [],
		activeImageIndex: 0,
		selectedLutPath: null,
		intensity: 1,
		phase: 'idle',
		result: null,
		previewUrl: null,
		previewPhase: 'idle',
		previewError: null,
		exportProgress: 0,
		exportStage: '',
		exportCompleted: 0,
		exportTotal: 0,
		exportResults: [],
		exportDestination: null,
		error: null
	};
}

export class AppContext {
	state = $state<AppState>(initialState());
	supportedLuts = $derived((this.state.catalog?.luts ?? []).filter((lut) => lut.supported));
	activeImage = $derived(this.state.images[this.state.activeImageIndex] ?? null);
	canProcess = $derived(
		this.state.images.length > 0 &&
		this.state.images.every((image) => image.kind === 'native') &&
		Boolean(this.state.selectedLutPath)
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
		releaseImagePreviews(this.state.images);
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

	async selectNativeImages(paths: string[]) {
		if (!paths.length) return [];
		const previous = this.state.images;
		this.clearPreview();
		const images = await this.run(() => selectNativeImages(paths), (images) => ({
			images,
			activeImageIndex: 0,
			...emptyExportState()
		}));
		releaseImagePreviews(previous);
		this.schedulePreview(0);
		return images;
	}

	async selectNativeImage(path: string) {
		return (await this.selectNativeImages([path]))[0] ?? null;
	}

	async chooseNativeImages() {
		const paths = await api.chooseImages();
		if (!paths.length) return null;
		return this.selectNativeImages(paths);
	}

	async chooseNativeImage() {
		return this.chooseNativeImages();
	}

	selectBrowserImages(files: File[]) {
		if (!files.length) return;
		this.clearPreview();
		const previous = this.state.images;
		this.state.images = selectBrowserImages(files);
		this.state.activeImageIndex = 0;
		Object.assign(this.state, emptyExportState());
		this.state.error = null;
		releaseImagePreviews(previous);
	}

	selectBrowserImage(file: File) {
		this.selectBrowserImages([file]);
	}

	selectImage(index: number) {
		if (index < 0 || index >= this.state.images.length || index === this.state.activeImageIndex) return;
		this.clearPreview();
		this.state.activeImageIndex = index;
		this.schedulePreview(0);
	}

	removeImage(index: number) {
		if (index < 0 || index >= this.state.images.length) return;
		this.clearPreview();
		const removed = this.state.images[index];
		this.state.images = this.state.images.filter((_, imageIndex) => imageIndex !== index);
		releaseImagePreview(removed);
		if (!this.state.images.length) {
			this.state.activeImageIndex = 0;
		} else if (index < this.state.activeImageIndex) {
			this.state.activeImageIndex -= 1;
		} else if (this.state.activeImageIndex >= this.state.images.length) {
			this.state.activeImageIndex = this.state.images.length - 1;
		}
		Object.assign(this.state, emptyExportState());
		this.schedulePreview(0);
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
		const image = this.activeImage;
		if (image?.kind !== 'native') {
			throw new Error('Choose an image in the desktop app before exporting.');
		}
		if (!this.state.selectedLutPath) throw new Error('Choose a LUT before exporting.');

		this.state.phase = 'processing';
		this.state.error = null;
		this.state.exportProgress = 5;
		this.state.exportStage = 'Preparing image';
		this.state.exportCompleted = 0;
		this.state.exportTotal = 1;
		this.state.exportResults = [];
		this.state.exportDestination = outputPath;
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
			this.state.exportResults = [result];
			this.state.exportCompleted = 1;
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

	async chooseAndProcessAll(suggestedName?: string): Promise<ProcessImageResult[] | null> {
		if (this.state.images.length <= 1) {
			const result = await this.chooseAndProcess(suggestedName);
			return result ? [result] : null;
		}
		const outputDirectory = await api.chooseOutputDirectory();
		if (!outputDirectory) return null;
		return this.processAll(outputDirectory);
	}

	async processAll(outputDirectory: string): Promise<ProcessImageResult[]> {
		const images = this.state.images;
		if (!images.length || images.some((image) => image.kind !== 'native')) {
			throw new Error('Choose images in the desktop app before exporting.');
		}
		const lutPath = this.state.selectedLutPath;
		if (!lutPath) throw new Error('Choose a LUT before exporting.');

		const outputPaths = await api.availableOutputPaths(
			outputDirectory,
			batchOutputNames(images, lutPath)
		);
		this.state.phase = 'processing';
		this.state.error = null;
		this.state.result = null;
		this.state.exportProgress = 0;
		this.state.exportCompleted = 0;
		this.state.exportTotal = images.length;
		this.state.exportResults = [];
		this.state.exportDestination = outputDirectory;
		try {
			for (const [index, image] of images.entries()) {
				if (image.kind !== 'native') continue;
				const outputPath = outputPaths[index];
				const result = await api.processImage({
					inputPath: image.path,
					lutPath,
					outputPath,
					intensity: this.state.intensity
				}, (progress) => {
					this.state.exportProgress = Math.round(((index + progress.percent / 100) / images.length) * 100);
					this.state.exportStage = `${index + 1} of ${images.length}: ${progress.stage}`;
				});
				this.state.exportResults = [...this.state.exportResults, result];
				this.state.exportCompleted = index + 1;
				this.state.result = result;
			}
			this.state.exportProgress = 100;
			this.state.exportStage = 'Export complete';
			this.state.phase = 'complete';
			return this.state.exportResults;
		} catch (error) {
			this.state.phase = 'error';
			this.state.error = errorMessage(error);
			throw error;
		}
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
		const image = this.activeImage;
		if (image?.kind !== 'native' || !this.state.selectedLutPath) return;
		this.previewTimer = setTimeout(() => void this.renderPreview(), delay);
	}

	private async renderPreview() {
		const image = this.activeImage;
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

function emptyExportState() {
	return {
		result: null,
		exportProgress: 0,
		exportStage: '',
		exportCompleted: 0,
		exportTotal: 0,
		exportResults: [],
		exportDestination: null
	} satisfies Partial<AppState>;
}

function batchOutputNames(images: SelectedImage[], lutPath: string): string[] {
	const lutName = fileStem(lutPath);
	const used = new Map<string, number>();
	return images.map((image) => {
		const sourceName = image.kind === 'native' ? fileStem(image.path) : fileStem(image.file.name);
		const base = cleanFileStem(`${sourceName}-${lutName}`);
		const count = (used.get(base) ?? 0) + 1;
		used.set(base, count);
		return `${base}${count > 1 ? `-${count}` : ''}.png`;
	});
}

function fileStem(path: string): string {
	return (path.split(/[\\/]/).pop() ?? 'image').replace(/\.[^.]+$/, '');
}

function cleanFileStem(value: string): string {
	return value.replace(/[<>:"/\\|?*\u0000-\u001f]/g, '-').replace(/[. ]+$/g, '') || 'image';
}

const [getAppContext, setAppContext] = createContext<AppContext>();

export { getAppContext };

export function provideAppContext() {
	return setAppContext(new AppContext());
}
