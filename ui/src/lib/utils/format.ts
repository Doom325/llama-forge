export function formatNumber(num: number): string {
	if (isNaN(num)) return '0';
	return new Intl.NumberFormat('en-US').format(num);
}

export function formatDuration(seconds: number): string {
	if (isNaN(seconds) || seconds < 0) return '0s';
	if (seconds === 0) return '0s';

	const m = Math.floor(seconds / 60);
	const s = Math.floor(seconds % 60);

	if (m > 0 && s > 0) return `${m}m ${s}s`;
	if (m > 0) return `${m}m`;
	return `${s}s`;
}

export function formatTimestamp(timestamp: string | number): string {
	const date = new Date(timestamp);
	if (isNaN(date.getTime())) return '';
	return new Intl.DateTimeFormat(undefined, {
		year: 'numeric',
		month: 'short',
		day: 'numeric',
		hour: '2-digit',
		minute: '2-digit',
		second: '2-digit'
	}).format(date);
}

export function formatBytes(bytes: number): string {
	if (isNaN(bytes) || bytes < 0) return '0 B';
	if (bytes === 0) return '0 B';

	const k = 1024;
	const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB', 'EB', 'ZB', 'YB'];
	const i = Math.floor(Math.log(bytes) / Math.log(k));
	return parseFloat((bytes / Math.pow(k, i)).toFixed(2)) + ' ' + (sizes[i] || 'B');
}

export function formatTokensPerSecond(tokens: number): string {
	if (isNaN(tokens)) return '0 tok/s';
	return `${parseFloat(tokens.toFixed(1))} tok/s`;
}
