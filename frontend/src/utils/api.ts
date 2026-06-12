import { Student, StudentProgress, DayAllocation } from '../App';

const apiRequest = async (endpoint: string, options: RequestInit = {}): Promise<any> => {
  const resp = await fetch(`/api${endpoint}`, {
    ...options,
    credentials: 'same-origin',
    headers: { 'Content-Type': 'application/json', ...options.headers },
  });
  const data = await resp.json().catch(() => ({}));
  if (!resp.ok) throw new Error(data.error || `HTTP ${resp.status}`);
  return data;
};

export const studentsAPI = {
  getAll: (): Promise<Student[]> =>
    apiRequest('/students').then(d => d.students ?? []),

  save: (students: Student[]): Promise<Student[]> =>
    apiRequest('/students', { method: 'POST', body: JSON.stringify({ students }) })
      .then(d => d.students ?? students),

  update: (id: string, student: Partial<Student>): Promise<Student> =>
    apiRequest(`/students/${id}`, { method: 'PUT', body: JSON.stringify(student) })
      .then(d => d.student),

  delete: (id: string): Promise<void> =>
    apiRequest(`/students/${id}`, { method: 'DELETE' }),
};

export const progressAPI = {
  getAll: (): Promise<StudentProgress[]> =>
    apiRequest('/progress').then(d => d.progressData ?? []),

  getByStudentId: (studentId: string): Promise<StudentProgress | null> =>
    apiRequest(`/progress/${studentId}`).then(d => d.progress ?? null),

  save: (progress: StudentProgress): Promise<StudentProgress> =>
    apiRequest('/progress', { method: 'POST', body: JSON.stringify(progress) })
      .then(d => d.progress ?? progress),

  delete: (studentId: string): Promise<void> =>
    apiRequest(`/progress/${studentId}`, { method: 'DELETE' }),
};

export const boatAllocationsAPI = {
  getAll: (): Promise<DayAllocation[]> =>
    apiRequest('/boat-allocations').then(d => d.boatAllocations ?? []),

  getByDay: (day: string): Promise<DayAllocation | null> =>
    apiRequest(`/boat-allocations/${encodeURIComponent(day)}`).then(d => d.allocation ?? null),

  save: (allocation: DayAllocation): Promise<DayAllocation> =>
    apiRequest('/boat-allocations', { method: 'POST', body: JSON.stringify(allocation) })
      .then(d => d.allocation ?? allocation),

  delete: (day: string): Promise<void> =>
    apiRequest(`/boat-allocations/${encodeURIComponent(day)}`, { method: 'DELETE' }),
};
