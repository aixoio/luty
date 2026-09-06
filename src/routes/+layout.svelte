<script lang="ts">
	import { onDestroy } from 'svelte';
	import { provideAppContext } from '$lib';
	import "./layout.css";

	let { children } = $props();
	const app = provideAppContext();
	onDestroy(() => app.reset());
</script>

{@render children()}
