export type ToastType = 'success' | 'error' | 'warning' | 'info';

export interface ToastMessage {
	id: string;
	type: ToastType;
	message: string;
}

class ToastStore {
	toasts = $state<ToastMessage[]>([]);
	private timeouts = new Map<string, ReturnType<typeof setTimeout>>();

	add(type: ToastType, message: string) {
		const id = crypto.randomUUID();
		this.toasts = [...this.toasts, { id, type, message }].slice(-3);

		const timeout = setTimeout(() => {
			this.remove(id);
		}, 5000);
		this.timeouts.set(id, timeout);
	}

	remove(id: string) {
		this.toasts = this.toasts.filter((t) => t.id !== id);
		const timeout = this.timeouts.get(id);
		if (timeout) {
			clearTimeout(timeout);
			this.timeouts.delete(id);
		}
	}
}

export const toastStore = new ToastStore();
