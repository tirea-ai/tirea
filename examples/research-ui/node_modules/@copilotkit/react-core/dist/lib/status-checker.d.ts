import { Severity } from '@copilotkit/shared';

type Status = {
    severity: Severity;
    message: string;
};
declare class StatusChecker {
    private activeKey;
    private intervalId;
    private instanceCount;
    private lastResponse;
    start(publicApiKey: string, onUpdate?: (status: Status | null) => void): Promise<Status | null | undefined>;
    getLastResponse(): Status | null;
    stop(): void;
}

export { Status, StatusChecker };
