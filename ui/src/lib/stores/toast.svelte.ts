export type ToastType = 'success' | 'error' | 'warning' | 'info';

export interface ToastMessage {
	id: string;
	type: ToastType;
	message: string;
}

class ToastStore {
	toasts = $state<ToastMessage[]>([]);

	add(type: ToastType, message: string) {
		const id = crypto.randomUUID();
		this.toasts = [...this.toasts, { id, type, message }].slice(-3);

		setTimeout(() => {
			this.remove(id);
		}, 5000);
	}

	remove(id: string) {
		this.toasts = this.toasts.filter((t) => t.id !== id);
	}
}

export const toastStore = new ToastStore();
