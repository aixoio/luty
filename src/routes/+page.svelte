<script lang="ts">
	import { onMount } from 'svelte';
	import { getAppContext, isNativeApp, listenForNativeImageDrops, type LutDescriptor } from '$lib';

	const app = getAppContext();

	let fileInput: HTMLInputElement;
	let settingsDialog: HTMLDialogElement;
	let onboardingDialog: HTMLDialogElement;
	let exportDialog: HTMLDialogElement;
	let query = $state('');
	let dragActive = $state(false);
	let hydrated = $state(false);
	let workspaceMode = $state<'single' | 'grid'>('single');
	let viewMode = $state<'before' | 'after'>('after');
	let wipeMode = $state(false);
	let holdingToCompare = $state(false);
	let holdStartedAt = 0;
	let suppressEyeClick = false;
	let visibleLuts = $derived(app.supportedLuts.filter((lut) => (lut.title ?? lut.name).toLowerCase().includes(query.trim().toLowerCase())));
	let activeLut = $derived(app.supportedLuts.find((lut) => lut.path === app.state.selectedLutPath) ?? null);
	let displayedImageUrl = $derived(
		viewMode === 'after' && app.state.previewUrl
			? app.state.previewUrl
			: app.activeImage?.previewUrl
	);

	$effect(() => {
		if (app.state.phase === 'processing' && exportDialog && !exportDialog.open) {
			exportDialog.showModal();
		}
	});

	onMount(() => {
		let disposed = false;
		let unlisten: () => void = () => {};
		void (async () => {
			try { if (isNativeApp()) await app.hydrate(); } catch { /* rendered by context */ }
			finally {
				hydrated = true;
				if (isNativeApp() && !app.state.settings.lutDirectory && !disposed) onboardingDialog?.showModal();
			}
			unlisten = await listenForNativeImageDrops(selectDroppedPaths, (value) => (dragActive = value));
		})();
		return () => { disposed = true; unlisten(); };
	});

	async function selectDroppedPaths(paths: string[]) {
		try { await app.selectNativeImages(paths); } catch { /* rendered by context */ }
	}

	async function chooseImage() {
		if (!isNativeApp()) { fileInput?.click(); return; }
		try { await app.chooseNativeImages(); } catch { /* rendered by context */ }
	}

	async function onBrowserFiles(event: Event) {
		const files = Array.from((event.currentTarget as HTMLInputElement).files ?? []);
		if (!files.length) return;
		try { await app.selectBrowserImages(files); } catch { /* rendered by context */ }
	}

	async function chooseLutFolder(closeOnSuccess = false) {
		try {
			const catalog = await app.chooseLutDirectory();
			if (catalog && closeOnSuccess) onboardingDialog?.close();
		} catch { /* rendered by store */ }
	}

	async function exportImage() {
		try {
			await app.chooseAndProcessAll(suggestedExportName());
		} catch { /* rendered by store */ }
	}
	async function exportCurrentImage() {
		try { await app.chooseAndProcess(suggestedExportName()); } catch { /* rendered by context */ }
	}
	function suggestedExportName() {
		const sourceName = app.activeImage?.kind === 'native'
			? app.activeImage.path.split(/[\\/]/).pop()?.replace(/\.[^.]+$/, '') : 'image';
		return `${sourceName ?? 'image'}-${activeLut?.name?.replace(/\.cube$/i, '') ?? 'graded'}.png`;
	}

	function lutLabel(lut: LutDescriptor) { return lut.title?.trim() || lut.name.replace(/\.cube$/i, ''); }
	function selectLut(path: string) {
		viewMode = app.activeImage?.kind === 'native' ? 'after' : 'before';
		app.selectLut(path);
	}
	function selectImage(index: number) {
		wipeMode = false;
		viewMode = app.state.images[index]?.kind === 'native' ? 'after' : 'before';
		app.selectImage(index);
	}
	function openImage(index: number) {
		selectImage(index);
		workspaceMode = 'single';
	}
	function selectRelativeImage(offset: number) {
		const total = app.state.images.length;
		if (total < 2) return;
		selectImage((app.state.activeImageIndex + offset + total) % total);
	}
	function handleFilmstripKeydown(event: KeyboardEvent) {
		const target = event.target instanceof HTMLElement
			? event.target.closest<HTMLElement>('[data-filmstrip-index]')
			: null;
		if (!target || app.state.images.length < 2) return;
		let nextIndex: number;
		switch (event.key) {
			case 'ArrowDown':
			case 'ArrowRight':
				nextIndex = (app.state.activeImageIndex + 1) % app.state.images.length;
				break;
			case 'ArrowUp':
			case 'ArrowLeft':
				nextIndex = (app.state.activeImageIndex - 1 + app.state.images.length) % app.state.images.length;
				break;
			case 'Home':
				nextIndex = 0;
				break;
			case 'End':
				nextIndex = app.state.images.length - 1;
				break;
			default:
				return;
		}
		event.preventDefault();
		event.stopPropagation();
		selectImage(nextIndex);
		requestAnimationFrame(() => {
			document.querySelector<HTMLButtonElement>(`[data-filmstrip-index="${nextIndex}"]`)?.focus();
		});
	}
	function handleImageShortcut(event: KeyboardEvent) {
		if (event.defaultPrevented || event.metaKey || event.ctrlKey || event.altKey || app.state.images.length < 2) return;
		if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
		const target = event.target instanceof HTMLElement ? event.target : null;
		if (target?.closest('button, input, textarea, select, a, [contenteditable="true"], dialog[open], .diff')) return;
		event.preventDefault();
		selectRelativeImage(event.key === 'ArrowRight' ? 1 : -1);
	}
	function removeImage(event: MouseEvent, index: number) {
		event.stopPropagation();
		wipeMode = false;
		app.removeImage(index);
	}
	function toggleWipe() {
		wipeMode = !wipeMode;
		viewMode = 'after';
	}
	function eyePointerDown(event: PointerEvent) {
		if (event.altKey) {
			event.preventDefault();
			toggleWipe();
			suppressEyeClick = true;
			return;
		}
		if (wipeMode || viewMode !== 'after') return;
		holdStartedAt = performance.now();
		holdingToCompare = true;
		viewMode = 'before';
		(event.currentTarget as HTMLButtonElement).setPointerCapture(event.pointerId);
	}
	function eyePointerUp() {
		if (!holdingToCompare) return;
		suppressEyeClick = performance.now() - holdStartedAt > 180;
		holdingToCompare = false;
		viewMode = 'after';
	}
	function eyeClick(event: MouseEvent) {
		if (suppressEyeClick) {
			suppressEyeClick = false;
			return;
		}
		if (event.altKey) {
			toggleWipe();
			return;
		}
		if (wipeMode) {
			wipeMode = false;
			return;
		}
		viewMode = viewMode === 'after' ? 'before' : 'after';
	}
	function cancelEyeHold() {
		if (!holdingToCompare) return;
		holdingToCompare = false;
		viewMode = 'after';
	}
	function imageName(image = app.activeImage) {
		if (!image) return 'No image selected';
		return image.kind === 'native' ? image.path.split(/[\\/]/).pop() ?? 'Image' : image.file.name;
	}
	function imageDimensions(image = app.activeImage) {
		return image?.kind === 'native' ? `${image.info.width} × ${image.info.height}` : 'Ready to preview';
	}
</script>

<svelte:head><title>Luty — fast LUT finishing</title><meta name="description" content="A fast, focused desktop LUT processor." /></svelte:head>
<svelte:window onkeydown={handleImageShortcut} />

<input bind:this={fileInput} type="file" class="hidden" multiple accept="image/png,image/jpeg,image/webp,image/avif,image/tiff,image/gif,image/bmp" onchange={onBrowserFiles} />

<main class="flex h-dvh min-h-[560px] flex-col overflow-hidden bg-base-200 text-base-content">
	<header class="flex h-13 shrink-0 items-center border-b border-base-300 bg-base-100 px-3 sm:px-4">
		<div class="flex min-w-0 flex-1 items-center gap-3">
			<div class="grid size-7 shrink-0 place-items-center rounded-field bg-base-content text-xs font-black text-base-200">L</div>
			<div class="hidden text-sm font-semibold sm:block">Luty</div><div class="h-4 w-px bg-base-300"></div>
			<p class="min-w-0 truncate text-sm font-medium">{imageName()}</p>
			{#if app.state.images.length > 1}
				<div class="flex shrink-0 items-center gap-0.5">
					<button class="btn btn-ghost btn-square btn-xs" aria-label="Previous image" onclick={() => selectRelativeImage(-1)}>‹</button>
					<span class="badge badge-ghost badge-sm">{app.state.activeImageIndex + 1} / {app.state.images.length}</span>
					<button class="btn btn-ghost btn-square btn-xs" aria-label="Next image" onclick={() => selectRelativeImage(1)}>›</button>
				</div>
			{/if}
			{#if app.state.phase === 'processing'}
				<span class="badge badge-soft badge-info badge-sm gap-1"><span class="loading loading-spinner loading-xs"></span>Processing</span>
			{:else if app.state.result}<span class="badge badge-soft badge-success badge-sm">{app.state.exportResults.length > 1 ? `${app.state.exportResults.length} images exported` : `Exported in ${app.state.result.elapsedMs} ms`}</span>{/if}
		</div>
		<div class="flex items-center gap-1">
			<button class="btn btn-ghost btn-sm hidden sm:inline-flex" onclick={chooseImage}>Open images</button>
			<div class="tooltip tooltip-bottom" data-tip="Settings">
				<button class="btn btn-ghost btn-square btn-sm" aria-label="Open settings" onclick={() => settingsDialog?.showModal()}>
					<svg viewBox="0 0 24 24" class="size-4" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="12" cy="12" r="3.5"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34A1.7 1.7 0 0 0 14 20.93V21h-4v-.09A1.7 1.7 0 0 0 9 19.36a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.62 15 1.7 1.7 0 0 0 3.07 14H3v-4h.09A1.7 1.7 0 0 0 4.64 9a1.7 1.7 0 0 0-.34-1.88l-.06-.06 2.83-2.83.06.06A1.7 1.7 0 0 0 9 4.62 1.7 1.7 0 0 0 10.07 3H14v.09A1.7 1.7 0 0 0 15 4.64a1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06A1.7 1.7 0 0 0 19.38 9 1.7 1.7 0 0 0 20.93 10H21v4h-.09A1.7 1.7 0 0 0 19.4 15Z"/></svg>
				</button>
			</div>
		</div>
	</header>

	{#if app.state.error}
		<div role="alert" class="alert alert-error alert-soft mx-3 mt-3 shrink-0 py-2 text-sm sm:mx-4">
			<svg viewBox="0 0 24 24" class="size-4" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><circle cx="12" cy="12" r="9"/><path d="M12 7v6m0 4h.01"/></svg>
			<span class="min-w-0 flex-1 truncate">{app.state.error}</span><button class="btn btn-ghost btn-xs" onclick={() => app.clearError()}>Dismiss</button>
		</div>
	{/if}

	<div class="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[224px_minmax(0,1fr)_292px]">
		<aside class="hidden min-h-0 border-r border-base-300 bg-base-100 lg:flex lg:flex-col">
			<div class="p-3">
				<button class="group flex w-full flex-col items-center justify-center gap-2 rounded-box border border-dashed p-5 text-center hover:border-base-content/40 hover:bg-base-200" class:border-primary={dragActive} class:bg-base-200={dragActive} onclick={chooseImage}>
					<svg viewBox="0 0 24 24" class="size-5 text-base-content/60" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><path d="M12 16V4m0 0L7.5 8.5M12 4l4.5 4.5"/><path d="M5 14v4a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2v-4"/></svg>
					<span class="text-sm font-medium">Drop or choose images</span><span class="text-xs text-base-content/45">Select one or many</span>
				</button>
			</div>
			<div class="flex min-h-0 flex-1 flex-col border-t border-base-300">
				<div class="flex items-center justify-between px-3 pb-1 pt-3"><h2 class="text-xs font-medium text-base-content/55">Images</h2><span class="badge badge-ghost badge-xs">{app.state.images.length}</span></div>
				{#if app.state.images.length}
					<ul class="list min-h-0 flex-1 overflow-y-auto p-2" aria-label="Loaded images">
						{#each app.state.images as image, index (image.previewUrl)}
							<li class="list-row gap-2 border border-transparent p-2" class:border-primary={index === app.state.activeImageIndex} class:bg-base-200={index === app.state.activeImageIndex}>
								<img src={image.previewUrl} alt="" class="size-9 rounded-field object-cover" class:ring-2={index === app.state.activeImageIndex} class:ring-primary={index === app.state.activeImageIndex} />
								<button class="min-w-0 text-left" data-filmstrip-index={index} tabindex={index === app.state.activeImageIndex ? 0 : -1} aria-current={index === app.state.activeImageIndex ? 'true' : undefined} onclick={() => selectImage(index)} onkeydown={handleFilmstripKeydown}>
									<span class="block truncate text-xs font-medium">{imageName(image)}</span>
									{#if index === app.state.activeImageIndex}
										<span class="mt-0.5 flex items-center gap-1 text-[10px] font-medium text-primary"><svg viewBox="0 0 16 16" class="size-3" fill="none" stroke="currentColor" stroke-width="2.2" aria-hidden="true"><path d="m3 8 3 3 7-7"/></svg>Selected</span>
									{:else}
										<span class="mt-0.5 block font-mono text-[10px] tabular-nums text-base-content/40">{imageDimensions(image)}</span>
									{/if}
								</button>
								<button class="btn btn-ghost btn-square btn-xs opacity-55 hover:opacity-100" aria-label={`Remove ${imageName(image)}`} onclick={(event) => removeImage(event, index)}>×</button>
							</li>
						{/each}
					</ul>
				{:else}
					<p class="px-4 py-3 text-xs text-base-content/40">No images loaded</p>
				{/if}
			</div>
			<div class="mt-auto border-t border-base-300 p-3">
				<div class="flex items-center justify-between text-xs"><span class="text-base-content/45">LUT folder</span><span class="badge badge-ghost badge-xs">{app.supportedLuts.length}</span></div>
				<p class="mt-2 truncate text-xs text-base-content/70">{app.state.settings.lutDirectory ?? 'Not connected'}</p>
				<button class="btn btn-ghost btn-sm mt-2 w-full" onclick={() => settingsDialog?.showModal()}>Manage library</button>
			</div>
		</aside>

		<section class="relative flex min-h-0 flex-col bg-base-200">
			<div class="flex h-11 shrink-0 items-center justify-between border-b border-base-300 bg-base-100 px-3 sm:px-4">
				<div role="tablist" class="tabs tabs-box tabs-sm" aria-label="Workspace view">
					<button role="tab" class="tab gap-1.5" class:tab-active={workspaceMode === 'single'} aria-selected={workspaceMode === 'single'} onclick={() => (workspaceMode = 'single')}>
						<svg viewBox="0 0 20 20" class="size-3.5" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><rect x="3" y="3" width="14" height="14" rx="2"/><path d="m4.5 14 4-4 3 3 2-2 2 2"/></svg>Single
					</button>
					<button role="tab" class="tab gap-1.5" class:tab-active={workspaceMode === 'grid'} aria-selected={workspaceMode === 'grid'} onclick={() => (workspaceMode = 'grid')}>
						<svg viewBox="0 0 20 20" class="size-3.5" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><rect x="3" y="3" width="5" height="5" rx="1"/><rect x="12" y="3" width="5" height="5" rx="1"/><rect x="3" y="12" width="5" height="5" rx="1"/><rect x="12" y="12" width="5" height="5" rx="1"/></svg>Grid
					</button>
				</div>
				<span class="text-xs text-base-content/45">{workspaceMode === 'grid' ? `${app.state.images.length} images` : imageName()}</span>
			</div>
			<div class="flex min-h-0 flex-1 items-center justify-center overflow-hidden p-3 sm:p-5">
				{#if app.state.images.length && workspaceMode === 'grid'}
					<div class="size-full overflow-y-auto rounded-box border border-base-300 bg-base-100 p-3 sm:p-4">
						<div class="grid grid-cols-2 gap-3 md:grid-cols-3 xl:grid-cols-4 2xl:grid-cols-5" role="listbox" aria-label="Loaded image grid">
							{#each app.state.images as image, index (image.previewUrl)}
								<button
									class="card card-sm card-border group overflow-hidden bg-base-200 text-left transition-colors"
									class:border-primary={index === app.state.activeImageIndex}
									class:ring-2={index === app.state.activeImageIndex}
									class:ring-primary={index === app.state.activeImageIndex}
									role="option"
									aria-selected={index === app.state.activeImageIndex}
									onclick={() => selectImage(index)}
									ondblclick={() => openImage(index)}
								>
									<figure class="relative aspect-4/3 overflow-hidden bg-base-300">
										<img src={image.previewUrl} alt={imageName(image)} class="size-full object-cover transition-transform duration-200 group-hover:scale-[1.02]" />
										{#if index === app.state.activeImageIndex}
											<span class="badge badge-primary badge-sm absolute left-2 top-2 gap-1"><svg viewBox="0 0 16 16" class="size-3" fill="none" stroke="currentColor" stroke-width="2.2" aria-hidden="true"><path d="m3 8 3 3 7-7"/></svg>Selected</span>
										{/if}
									</figure>
									<div class="card-body gap-1 p-3">
										<h2 class="card-title block truncate text-sm">{imageName(image)}</h2>
										<p class="font-mono text-[10px] tabular-nums text-base-content/45">{imageDimensions(image)}</p>
									</div>
								</button>
							{/each}
						</div>
					</div>
				{:else if app.activeImage}
					<div class="relative flex size-full items-center justify-center overflow-hidden rounded-box border border-base-300 bg-base-100">
						{#if wipeMode && app.state.previewUrl}
							<!-- svelte-ignore a11y_no_noninteractive_tabindex (required by DaisyUI diff keyboard interaction) -->
							<figure class="diff size-full bg-base-100" tabindex="0" aria-label="Draggable before and after comparison">
								<!-- svelte-ignore a11y_no_noninteractive_tabindex (required by DaisyUI diff keyboard interaction) -->
								<div class="diff-item-1 bg-base-100" role="img" tabindex="0" aria-label={`Before, original ${imageName()}`}>
									<img src={app.activeImage.previewUrl} alt="" style="object-fit: contain" />
								</div>
								<div class="diff-item-2 bg-base-100" role="img" aria-label={`After preview of ${imageName()}`}>
									<img src={app.state.previewUrl} alt="" style="object-fit: contain" />
								</div>
								<div class="diff-resizer"></div>
							</figure>
							<div class="pointer-events-none absolute left-3 top-3 z-10"><span class="badge badge-neutral badge-sm">Before · original</span></div>
							<div class="pointer-events-none absolute right-3 top-3 z-10"><span class="badge badge-neutral badge-sm">After · {activeLut ? lutLabel(activeLut) : 'graded'}</span></div>
						{:else}
							<img src={displayedImageUrl} alt={`${viewMode === 'after' ? 'After' : 'Before'} preview of ${imageName()}`} class="max-h-full max-w-full object-contain" />
							<div class="pointer-events-none absolute left-3 top-3 flex items-center gap-2"><span class="badge badge-neutral badge-sm">{viewMode === 'before' ? 'Before · original' : activeLut ? `After · ${lutLabel(activeLut)}` : 'Original'}</span>{#if activeLut && viewMode === 'after'}<span class="badge badge-ghost badge-sm font-mono tabular-nums">{Math.round(app.state.intensity * 100)}%</span>{/if}</div>
						{/if}
						{#if app.state.previewPhase === 'rendering' && viewMode === 'after'}
							<div class="pointer-events-none absolute inset-0 grid place-items-center bg-base-200/35"><span class="badge badge-neutral gap-2"><span class="loading loading-spinner loading-xs"></span>Rendering preview</span></div>
						{/if}
						{#if app.state.previewError && viewMode === 'after'}
							<div role="alert" class="alert alert-error alert-soft absolute bottom-16 left-1/2 w-auto max-w-[80%] -translate-x-1/2 py-2 text-xs"><span>{app.state.previewError}</span></div>
						{/if}
						<div class="tooltip tooltip-bottom absolute left-1/2 top-3 z-20 -translate-x-1/2" data-tip="Click toggles · Hold for before · ⌥-click for slider">
							<button
								class="btn btn-circle btn-neutral border-base-300"
								class:btn-active={wipeMode || viewMode === 'after'}
								aria-label={wipeMode ? 'Exit before and after slider' : viewMode === 'after' ? 'Hide the LUT preview' : 'Show the LUT preview'}
								aria-pressed={wipeMode || viewMode === 'after'}
								disabled={!activeLut || app.activeImage.kind !== 'native'}
								onpointerdown={eyePointerDown}
								onpointerup={eyePointerUp}
								onpointercancel={cancelEyeHold}
								onclick={eyeClick}
							>
								{#if viewMode === 'before' && !wipeMode}
									<svg viewBox="0 0 24 24" class="size-5" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M3 3l18 18M10.6 10.7a2 2 0 0 0 2.7 2.7M9.9 4.3A10.6 10.6 0 0 1 12 4c5.5 0 9 5 9 5a16 16 0 0 1-2.1 2.5M6.6 6.7C4.4 8.2 3 10 3 10s3.5 5 9 5a10 10 0 0 0 3.2-.5"/></svg>
								{:else}
									<svg viewBox="0 0 24 24" class="size-5" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><path d="M3 12s3.5-5 9-5 9 5 9 5-3.5 5-9 5-9-5-9-5Z"/><circle cx="12" cy="12" r="2.5"/>{#if wipeMode}<path d="M12 5v14"/>{/if}</svg>
								{/if}
							</button>
						</div>
					</div>
				{:else}
					<button class="flex h-full min-h-72 w-full max-w-2xl flex-col items-center justify-center rounded-box border border-dashed border-base-300 bg-base-100 p-8 text-center hover:border-base-content/40" class:border-primary={dragActive} onclick={chooseImage}>
						<div class="mb-5 grid size-14 place-items-center rounded-full border border-base-300 bg-base-200"><svg viewBox="0 0 24 24" class="size-6 text-base-content/65" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true"><rect x="3" y="4" width="18" height="16" rx="2"/><circle cx="8.5" cy="9" r="1.5"/><path d="m4 17 5-5 4 4 2-2 5 4"/></svg></div>
						<h1 class="text-xl font-semibold tracking-tight">Drop images to begin</h1><p class="mt-2 max-w-sm text-sm leading-6 text-base-content/50">Choose one photo or a full set. Luty processes every pixel locally.</p><span class="btn btn-primary btn-sm mt-6">Choose images</span>
					</button>
				{/if}
			</div>
			<div class="flex h-12 shrink-0 items-center justify-between border-t border-base-300 bg-base-100 px-3 text-xs sm:px-4"><div class="flex items-center gap-2 text-base-content/50"><span class="status" class:status-success={app.activeImage !== null}></span><span>{app.activeImage ? imageDimensions() : 'Waiting for images'}</span></div><span class="font-mono tabular-nums text-base-content/45">{workspaceMode === 'grid' ? `Grid · ${app.state.images.length}` : 'Fit · 100%'}</span></div>
		</section>

		<aside class="flex min-h-0 flex-col border-l border-base-300 bg-base-100 max-lg:absolute max-lg:bottom-12 max-lg:right-0 max-lg:top-13 max-lg:z-10 max-lg:w-[272px] max-lg:translate-x-[calc(100%-44px)] max-lg:shadow-2xl max-lg:transition-transform max-lg:hover:translate-x-0">
			<div class="border-b border-base-300 p-3"><div class="mb-3 flex items-center justify-between"><h2 class="text-sm font-semibold">Looks</h2><span class="text-xs text-base-content/45">{visibleLuts.length} LUTs</span></div><label class="input input-sm w-full bg-base-200"><svg viewBox="0 0 24 24" class="size-4 text-base-content/40" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="11" cy="11" r="7"/><path d="m20 20-4-4"/></svg><input bind:value={query} type="search" placeholder="Search LUTs" aria-label="Search LUTs" /></label></div>
			<div class="min-h-0 flex-1 overflow-y-auto p-3">
				{#if !hydrated || app.state.phase === 'loading'}
					<div class="grid grid-cols-2 gap-2">{#each Array(6) as _}<div class="skeleton h-24 rounded-box"></div>{/each}</div>
				{:else if visibleLuts.length}
					<div class="grid grid-cols-2 gap-2" role="listbox" aria-label="LUT library">
						{#each visibleLuts as lut}
							<button class="group overflow-hidden rounded-box border bg-base-200 text-left" class:border-base-300={app.state.selectedLutPath !== lut.path} class:border-primary={app.state.selectedLutPath === lut.path} class:ring-1={app.state.selectedLutPath === lut.path} class:ring-primary={app.state.selectedLutPath === lut.path} role="option" aria-selected={app.state.selectedLutPath === lut.path} onclick={() => selectLut(lut.path)}>
								<div class="relative h-16 overflow-hidden bg-base-300">{#if app.activeImage}<img src={app.activeImage.previewUrl} alt="" class="size-full object-cover opacity-75 transition-transform duration-200 group-hover:scale-105" />{:else}<div class="grid size-full place-items-center text-[10px] text-base-content/25">Preview</div>{/if}{#if app.state.selectedLutPath === lut.path}<span class="absolute right-1.5 top-1.5 grid size-4 place-items-center rounded-full bg-primary text-primary-content"><svg viewBox="0 0 16 16" class="size-3" fill="none" stroke="currentColor" stroke-width="2"><path d="m3 8 3 3 7-7"/></svg></span>{/if}</div>
								<div class="px-2 py-2"><p class="truncate text-xs font-medium">{lutLabel(lut)}</p><p class="mt-0.5 text-[10px] text-base-content/40">{lut.size ? `${lut.size} point` : 'Cube LUT'}</p></div>
							</button>
						{/each}
					</div>
				{:else}
					<div class="flex h-full min-h-44 flex-col items-center justify-center px-3 text-center"><p class="text-sm font-medium">{query ? 'No matching LUTs' : 'Your LUT library is empty'}</p><p class="mt-1 text-xs leading-5 text-base-content/45">{query ? 'Try a different search.' : 'Choose a folder containing .cube files.'}</p>{#if !query}<button class="btn btn-ghost btn-sm mt-3" onclick={() => chooseLutFolder()}>Choose folder</button>{/if}</div>
				{/if}
			</div>
			<div class="shrink-0 border-t border-base-300 p-3">
				<div class="mb-2 flex items-center justify-between text-xs"><label for="lut-strength" class="font-medium">Strength</label><span class="font-mono tabular-nums text-base-content/50">{Math.round(app.state.intensity * 100)}%</span></div>
				<input id="lut-strength" type="range" min="0" max="100" value={Math.round(app.state.intensity * 100)} class="range range-xs w-full" disabled={!activeLut} oninput={(event) => app.setIntensity(Number((event.currentTarget as HTMLInputElement).value) / 100)} />
				<button class="btn btn-block mt-3" class:btn-primary={app.canProcess} disabled={!app.canProcess || app.state.phase === 'processing'} onclick={exportImage}>{#if app.state.phase === 'processing'}<span class="loading loading-spinner loading-sm"></span>{/if}{app.state.phase === 'processing' ? `Exporting ${app.state.exportCompleted + 1} of ${app.state.exportTotal}…` : app.state.images.length > 1 ? `Export ${app.state.images.length} images` : 'Export image'}</button>
				{#if app.state.images.length > 1}<button class="btn btn-ghost btn-sm btn-block mt-1" disabled={!app.canProcess || app.state.phase === 'processing'} onclick={exportCurrentImage}>Export current only</button>{/if}
			</div>
		</aside>
	</div>
</main>

<dialog bind:this={onboardingDialog} class="modal modal-middle">
	<div class="modal-box max-w-md border border-base-300 bg-base-100 p-0">
		<div class="border-b border-base-300 p-6"><div class="mb-5 grid size-10 place-items-center rounded-field bg-base-content text-sm font-black text-base-200">L</div><h2 class="text-xl font-semibold tracking-tight">Connect your LUT library</h2><p class="mt-2 text-sm leading-6 text-base-content/55">Choose the folder where you keep .cube LUT files. Luty scans it locally and remembers it for next time.</p></div>
		<div class="p-6">{#if app.state.error}<p class="mb-4 text-sm text-error">{app.state.error}</p>{/if}<button class="btn btn-primary btn-block" onclick={() => chooseLutFolder(true)} disabled={app.state.phase === 'loading'}>{#if app.state.phase === 'loading'}<span class="loading loading-spinner loading-sm"></span>{/if}Choose LUT folder</button><form method="dialog" class="mt-2"><button class="btn btn-ghost btn-block">Set up later</button></form></div>
	</div><form method="dialog" class="modal-backdrop"><button aria-label="Close onboarding">close</button></form>
</dialog>

<dialog bind:this={settingsDialog} class="modal modal-middle">
	<div class="modal-box max-w-lg border border-base-300 bg-base-100">
		<div class="flex items-start justify-between gap-4"><div><h2 class="text-lg font-semibold">Settings</h2><p class="mt-1 text-sm text-base-content/50">Luty stores these preferences on this device.</p></div><form method="dialog"><button class="btn btn-ghost btn-square btn-sm" aria-label="Close settings">✕</button></form></div>
		<div class="mt-6 rounded-box border border-base-300 bg-base-200 p-4"><div class="flex items-start justify-between gap-4"><div class="min-w-0"><p class="text-sm font-medium">LUT library</p><p class="mt-1 truncate text-xs text-base-content/45">{app.state.settings.lutDirectory ?? 'No folder selected'}</p></div><button class="btn btn-sm shrink-0" onclick={() => chooseLutFolder()}>Change</button></div><div class="mt-3 flex items-center gap-2 text-xs text-base-content/50"><span class="status" class:status-success={app.supportedLuts.length > 0}></span>{app.supportedLuts.length} supported LUTs found</div></div>
		<div class="modal-action"><form method="dialog"><button class="btn">Done</button></form></div>
	</div><form method="dialog" class="modal-backdrop"><button aria-label="Close settings">close</button></form>
</dialog>

<dialog bind:this={exportDialog} class="modal modal-middle" aria-labelledby="export-title">
	<div class="modal-box max-w-md border border-base-300 bg-base-100">
		{#if app.state.phase === 'processing'}
			<div class="flex items-center gap-3">
				<span class="loading loading-spinner loading-md text-primary"></span>
				<div>
					<h2 id="export-title" class="text-lg font-semibold">{app.state.exportTotal > 1 ? `Exporting ${app.state.exportCompleted + 1} of ${app.state.exportTotal}` : 'Exporting image'}</h2>
					<p class="mt-1 text-sm text-base-content/50">Keep Luty open while {app.state.exportTotal > 1 ? 'the image queue is' : 'the full-resolution image is'} processed.</p>
				</div>
			</div>
			<div class="mt-6 flex items-center justify-between text-xs">
				<span>{app.state.exportStage || 'Preparing image'}</span>
				<span class="font-mono tabular-nums text-base-content/55">{app.state.exportProgress}%</span>
			</div>
			<progress class="progress progress-primary mt-2 w-full" value={app.state.exportProgress} max="100"></progress>
		{:else if app.state.result}
			<div class="flex items-start gap-3">
				<span class="grid size-9 shrink-0 place-items-center rounded-full bg-success text-success-content">
					<svg viewBox="0 0 20 20" class="size-5" fill="none" stroke="currentColor" stroke-width="2" aria-hidden="true"><path d="m4 10 4 4 8-8"/></svg>
				</span>
				<div class="min-w-0">
					<h2 id="export-title" class="text-lg font-semibold">{app.state.exportResults.length > 1 ? `${app.state.exportResults.length} images exported` : 'Export complete'}</h2>
					<p class="mt-1 truncate text-sm text-base-content/50">{app.state.exportDestination ?? app.state.result.outputPath}</p>
					{#if app.state.exportResults.length === 1}<p class="mt-2 font-mono text-xs tabular-nums text-base-content/40">{app.state.result.width} × {app.state.result.height} · {app.state.result.elapsedMs} ms</p>{/if}
				</div>
			</div>
			<progress class="progress progress-success mt-6 w-full" value="100" max="100"></progress>
			<div class="modal-action"><form method="dialog"><button class="btn">Done</button></form></div>
		{:else}
			<h2 id="export-title" class="text-lg font-semibold">Export stopped</h2>
			<p class="mt-2 text-sm text-error">{app.state.error ?? 'The image could not be exported.'}</p>
			{#if app.state.exportCompleted > 0}<p class="mt-2 text-xs text-base-content/50">{app.state.exportCompleted} of {app.state.exportTotal} images were exported before the error.</p>{/if}
			<progress class="progress progress-error mt-6 w-full" value={app.state.exportProgress} max="100"></progress>
			<div class="modal-action"><form method="dialog"><button class="btn">Close</button></form></div>
		{/if}
	</div>
</dialog>
