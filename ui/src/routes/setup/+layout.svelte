<script lang="ts">
	import type { Snippet } from 'svelte';
	import { page } from '$app/state';
	import { goto, afterNavigate } from '$app/navigation';
	import { base } from '$app/paths';
	import { queryClient } from '$lib/api/queryClient';
	import StepIndicator from '$lib/components/setup/StepIndicator.svelte';
	import StepNavigation from '$lib/components/setup/StepNavigation.svelte';
	import {
		SetupWizardStore,
		provideSetupStore,
		WIZARD_STEPS,
		STEP_TRANSITIONS,
		completedSteps,
		invalidateDownstream
	} from '$lib/stores/setup.svelte';
	import type { WizardStep } from '$lib/api/types';

	let { children }: { children: Snippet } = $props();

	// One store per wizard session, provided to all steps via context.
	const store = new SetupWizardStore(crypto.randomUUID());
	provideSetupStore(store);

	const STEP_LABELS: Record<WizardStep, string> = {
		welcome: 'Welcome',
		hardware: 'Hardware',
		binary: 'Binary',
		models: 'Models',
		configure: 'Configure',
		review: 'Review'
	};

	function stepPath(step: WizardStep): string {
		return step === 'welcome' ? `${base}/setup` : `${base}/setup/${step}`;
	}

	function stepFromPath(pathname: string): WizardStep {
		const seg = pathname.split('/setup')[1]?.replace(/^\//, '').split('/')[0] ?? '';
		return WIZARD_STEPS.find((s) => s === seg) ?? 'welcome';
	}

	const currentStep = $derived(stepFromPath(page.url.pathname));
	const currentIndex = $derived(WIZARD_STEPS.indexOf(currentStep));
	const configureIndex = WIZARD_STEPS.indexOf('configure');

	// The step guards read server data via queryClient.getQueryData(), which is
	// NOT a Svelte-reactive source — so re-evaluating them on store changes alone
	// would miss query results landing. Bump a counter on every cache change and
	// reference it in the cache-reading deriveds to keep them reactive.
	let cacheVersion = $state(0);
	$effect(() => {
		// Bump ONLY on query-data state changes (fetch success / setQueryData),
		// NOT on observer events — those fire on every re-render and would feed
		// back into the bump, causing effect_update_depth_exceeded.
		const unsubscribe = queryClient.getQueryCache().subscribe((event) => {
			if (event.type === 'updated') cacheVersion += 1;
		});
		return unsubscribe;
	});

	// completedSteps is *derived* from the transition guards, never stored.
	const done = $derived.by(() => {
		void cacheVersion;
		return completedSteps(queryClient, store);
	});

	const indicatorSteps = $derived(
		WIZARD_STEPS.map((step, index) => ({
			label: STEP_LABELS[step],
			status: (step === currentStep
				? 'current'
				: done.has(step) && index < currentIndex
					? 'complete'
					: 'upcoming') as 'complete' | 'current' | 'upcoming'
		}))
	);

	const canAdvance = $derived.by(() => {
		void cacheVersion;
		return STEP_TRANSITIONS[currentStep].canAdvance(queryClient, store);
	});

	function navigateTo(index: number) {
		const target = WIZARD_STEPS[index];
		if (!target) return;
		// Re-entering an earlier step invalidates downstream server data + state.
		invalidateDownstream(queryClient, store, target);
		goto(stepPath(target));
	}

	function next() {
		if (currentIndex < WIZARD_STEPS.length - 1) navigateTo(currentIndex + 1);
	}

	function back() {
		if (currentIndex > 0) navigateTo(currentIndex - 1);
	}

	// Warn before leaving once the user has invested configuration work.
	$effect(() => {
		if (currentIndex < configureIndex) return;
		const handler = (event: BeforeUnloadEvent) => {
			event.preventDefault();
			event.returnValue = '';
		};
		window.addEventListener('beforeunload', handler);
		return () => window.removeEventListener('beforeunload', handler);
	});

	// Move focus to the new step's heading after navigation (a11y).
	afterNavigate(() => {
		document.querySelector<HTMLElement>('main h1[tabindex="-1"]')?.focus();
	});
</script>

<svelte:head>
	<title>Setup — LlamaForge</title>
</svelte:head>

<div class="mx-auto max-w-3xl space-y-8 px-4 py-8">
	<StepIndicator steps={indicatorSteps} onnavigate={navigateTo} />

	<main>
		{@render children()}
	</main>

	{#if currentStep !== 'welcome'}
		<StepNavigation
			canGoBack={currentIndex > 0}
			{canAdvance}
			nextLabel={currentStep === 'review' ? 'Finish' : 'Next'}
			onback={back}
			onnext={next}
		/>
	{/if}
</div>
