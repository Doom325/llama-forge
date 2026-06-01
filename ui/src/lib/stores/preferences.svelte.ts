export type Theme = 'light' | 'dark' | 'system';

class PreferencesStore {
	theme = $state<Theme>('system');
	sidebarCollapsed = $state<boolean>(false);

	constructor() {
		if (typeof localStorage !== 'undefined') {
			const storedTheme = localStorage.getItem('theme') as Theme | null;
			if (storedTheme) {
				this.theme = storedTheme;
			}
			const storedSidebar = localStorage.getItem('sidebarCollapsed');
			if (storedSidebar !== null) {
				this.sidebarCollapsed = storedSidebar === 'true';
			}
		}
	}

	setTheme(newTheme: Theme) {
		this.theme = newTheme;
		if (typeof localStorage !== 'undefined') {
			localStorage.setItem('theme', newTheme);
		}
	}

	toggleTheme() {
		if (this.theme === 'system') {
			if (typeof window !== 'undefined' && window.matchMedia) {
				const prefersDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
				this.setTheme(prefersDark ? 'light' : 'dark');
			} else {
				this.setTheme('dark');
			}
		} else {
			this.setTheme(this.theme === 'light' ? 'dark' : 'light');
		}
	}

	toggleSidebar() {
		this.sidebarCollapsed = !this.sidebarCollapsed;
		if (typeof localStorage !== 'undefined') {
			localStorage.setItem('sidebarCollapsed', String(this.sidebarCollapsed));
		}
	}
}

export const preferences = new PreferencesStore();
