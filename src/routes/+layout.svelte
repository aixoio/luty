<script lang="ts">
    import type { PageProps } from "./$types";
    import "./layout.css";

    const { children }: PageProps = $props();
</script>

{@render children()}
