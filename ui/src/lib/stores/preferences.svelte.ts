export type Theme = 'light' | 'dark' | 'system';

class PreferencesStore {
	theme = $state<Theme>('system');
	sidebarCollapsed = $state<boolean>(false);

	constructor() {
		if (typeof localStorage !== 'undefined') {
			const storedTheme = localStorage.getItem('theme');
			if (storedTheme === 'light' || storedTheme === 'dark' || storedTheme === 'system') {
				this.theme = storedTheme as Theme;
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
			this.setTheme('light');
		} else if (this.theme === 'light') {
			this.setTheme('dark');
		} else {
			this.setTheme('system');
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
