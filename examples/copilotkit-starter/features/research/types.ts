export interface Resource {
  id: string;
  url: string;
  title: string;
  description: string;
}

export interface LogEntry {
  message: string;
  level: string;
  step: string;
}

export interface ResearchState {
  research_question: string;
  report: string;
  resources: Resource[];
  logs: LogEntry[];
}
