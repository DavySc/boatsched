import React, { useState, useEffect } from 'react';
import { Student, BoatTable, DayAllocation } from '../App';
import { Card, CardContent, CardHeader, CardTitle } from './ui/card';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Badge } from './ui/badge';
import { Checkbox } from './ui/checkbox';
import { Popover, PopoverContent, PopoverTrigger } from './ui/popover';
import { ChevronDown, X } from 'lucide-react';
import { toast } from 'sonner';

interface BoatAllocationProps {
  day: string;
  students: Student[]; // Already sorted alphabetically from App.tsx
  existingAllocation?: DayAllocation;
  onAllocationUpdate: (allocation: DayAllocation) => void;
}

const levelOptions = ['Ketelbinkie', 'Bemanning', 'A', 'B', 'Heterogeen'];

// Migration helper to convert old format to new format
const migrateOldBoatTable = (oldTable: any): BoatTable => {
  // If it's already in the new format, return as is
  if (oldTable.selectedStudents !== undefined) {
    return oldTable;
  }

  // Convert old format with studentRows to new format with selectedStudents
  const selectedStudents: string[] = [];
  if (oldTable.studentRows && Array.isArray(oldTable.studentRows)) {
    oldTable.studentRows.forEach((row: string[]) => {
      if (Array.isArray(row)) {
        selectedStudents.push(...row);
      }
    });
  }

  return {
    id: oldTable.id || `table-${Date.now()}`,
    selectedLevels: oldTable.selectedLevels || [],
    boatAndSkipperName: oldTable.boatAndSkipperName || oldTable.boatName || '',
    selectedStudents: selectedStudents
  };
};

export function BoatAllocation({ day, students, existingAllocation, onAllocationUpdate }: BoatAllocationProps) {
  const [tables, setTables] = useState<BoatTable[]>([]);
  const [isViewMode, setIsViewMode] = useState(false);

  // Initialize tables with migration support
  useEffect(() => {
    if (existingAllocation && existingAllocation.tables) {
      // Migrate old format to new format if needed
      const migratedTables = existingAllocation.tables.map(migrateOldBoatTable);
      setTables(migratedTables);
      setIsViewMode(existingAllocation.isViewMode || false);
    } else {
      // Create 13 empty tables
      const initialTables: BoatTable[] = Array.from({ length: 13 }, (_, index) => ({
        id: `table-${index + 1}`,
        selectedLevels: [],
        boatAndSkipperName: '',
        selectedStudents: [] // Single row for students
      }));
      setTables(initialTables);
      setIsViewMode(false);
    }
  }, [existingAllocation]);

  // Get all selected students across all tables to prevent duplicates
  const getAllSelectedStudents = () => {
    const selectedStudents = new Set<string>();
    if (tables && Array.isArray(tables)) {
      tables.forEach(table => {
        if (table && table.selectedStudents && Array.isArray(table.selectedStudents)) {
          table.selectedStudents.forEach(studentId => {
            if (studentId) selectedStudents.add(studentId);
          });
        }
      });
    }
    return selectedStudents;
  };

  const updateTable = (tableId: string, updates: Partial<BoatTable>) => {
    setTables(prev => prev.map(table => 
      table.id === tableId ? { ...table, ...updates } : table
    ));
  };

  const toggleLevel = (tableId: string, level: string) => {
    const table = tables.find(t => t.id === tableId);
    if (!table) return;

    const updatedLevels = table.selectedLevels.includes(level)
      ? table.selectedLevels.filter(l => l !== level)
      : [...table.selectedLevels, level];
    
    updateTable(tableId, { selectedLevels: updatedLevels });
  };

  const toggleStudent = (tableId: string, studentId: string) => {
    const table = tables.find(t => t.id === tableId);
    if (!table) return;

    const updatedStudents = table.selectedStudents.includes(studentId)
      ? table.selectedStudents.filter(id => id !== studentId)
      : [...table.selectedStudents, studentId];
    
    updateTable(tableId, { selectedStudents: updatedStudents });
  };

  const removeStudent = (tableId: string, studentId: string) => {
    const table = tables.find(t => t.id === tableId);
    if (!table) return;

    const updatedStudents = table.selectedStudents.filter(id => id !== studentId);
    updateTable(tableId, { selectedStudents: updatedStudents });
  };

  const saveAllocation = () => {
    const allocation: DayAllocation = {
      day,
      tables,
      isViewMode: true
    };
    
    onAllocationUpdate(allocation);
    setIsViewMode(true);
    toast.success('Bootindeling opgeslagen!');
  };

  const toggleEditMode = () => {
    if (isViewMode) {
      setIsViewMode(false);
    } else {
      saveAllocation();
    }
  };

  // Helper function to convert level name to number
  const getLevelNumberFromName = (levelName: string): number | null => {
    const levelMap: { [key: string]: number } = {
      'Ketelbinkie': 1,
      'Bemanning': 2,
      'A': 3,
      'B': 4
    };
    return levelMap[levelName] || null;
  };

  // Updated function to filter students based on selected levels
  const getAvailableStudents = (currentTableId: string) => {
    const allSelected = getAllSelectedStudents();
    const currentTable = tables.find(t => t.id === currentTableId);
    const currentStudents = currentTable?.selectedStudents || [];
    
    // First filter: exclude already selected students (except those in current table)
    let availableStudents = students.filter(student => 
      !allSelected.has(student.id) || currentStudents.includes(student.id)
    );

    // Second filter: filter by selected levels
    if (currentTable?.selectedLevels && currentTable.selectedLevels.length > 0) {
      // If "Heterogeen" is selected, show all students
      if (currentTable.selectedLevels.includes('Heterogeen')) {
        return availableStudents; // No level filtering, show all available students
      }
      
      // Convert selected level names to numbers and filter students
      const selectedLevelNumbers = currentTable.selectedLevels
        .map(getLevelNumberFromName)
        .filter(level => level !== null) as number[];
      
      if (selectedLevelNumbers.length > 0) {
        availableStudents = availableStudents.filter(student => 
          selectedLevelNumbers.includes(student.level)
        );
      }
    }
    
    return availableStudents;
  };

  const getLevelName = (levelNumber: number) => {
    const levelMap = {
      1: 'Ketelbinkie',
      2: 'Bemanning',
      3: 'A',
      4: 'B'
    };
    return levelMap[levelNumber as keyof typeof levelMap] || `Level ${levelNumber}`;
  };

  // Add defensive check for tables
  if (!tables || !Array.isArray(tables)) {
    return <div>Loading...</div>;
  }

  return (
    <div className="space-y-4">
      {/* Header with save/edit button */}
      <div className="flex justify-end">
        <Button 
          onClick={toggleEditMode}
          className={isViewMode 
            ? "text-white hover:opacity-90" 
            : "text-white hover:opacity-90"
          }
          style={{ backgroundColor: isViewMode ? '#E60000' : '#E60000' }}
        >
          {isViewMode ? 'Bewerken' : 'Opslaan'}
        </Button>
      </div>

      {/* Tables Grid */}
      <div className="space-y-4">
        {tables.map((table, index) => (
          <Card key={table.id}>
            <CardHeader className="pb-3">
              <CardTitle className="text-lg">Boot {index + 1}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-4">
              {/* Row 1: Level Selection */}
              <div className="border rounded-lg p-3">
                <div className="text-sm font-medium mb-2">Niveaus</div>
                {isViewMode ? (
                  <div className="flex flex-wrap gap-1">
                    {(table.selectedLevels || []).map(level => (
                      <Badge key={level} variant="secondary">{level}</Badge>
                    ))}
                    {(!table.selectedLevels || table.selectedLevels.length === 0) && (
                      <span className="text-muted-foreground text-sm">Geen niveaus geselecteerd</span>
                    )}
                  </div>
                ) : (
                  <Popover>
                    <PopoverTrigger asChild>
                      <Button variant="outline" className="w-full justify-between">
                        {(table.selectedLevels && table.selectedLevels.length > 0)
                          ? `${table.selectedLevels.length} niveau(s) geselecteerd`
                          : "Selecteer niveaus"
                        }
                        <ChevronDown className="h-4 w-4" />
                      </Button>
                    </PopoverTrigger>
                    <PopoverContent className="w-64 p-3">
                      <div className="space-y-2">
                        {levelOptions.map(level => (
                          <div key={level} className="flex items-center space-x-2">
                            <Checkbox
                              id={`${table.id}-${level}`}
                              checked={(table.selectedLevels || []).includes(level)}
                              onCheckedChange={() => toggleLevel(table.id, level)}
                            />
                            <label htmlFor={`${table.id}-${level}`} className="text-sm">
                              {level}
                            </label>
                          </div>
                        ))}
                      </div>
                    </PopoverContent>
                  </Popover>
                )}
              </div>

              {/* Row 2: Boot en schipper */}
              <div className="border rounded-lg p-3">
                <div className="text-sm font-medium mb-2">Boot en schipper</div>
                {isViewMode ? (
                  <div className="text-sm">
                    {table.boatAndSkipperName || <span className="text-muted-foreground">Geen naam ingevuld</span>}
                  </div>
                ) : (
                  <Input
                    value={table.boatAndSkipperName || ''}
                    onChange={(e) => updateTable(table.id, { boatAndSkipperName: e.target.value })}
                    placeholder="Voer boot en schipper naam in"
                  />
                )}
              </div>

              {/* Row 3: Student Selection */}
              <div className="border rounded-lg p-3">
                <div className="text-sm font-medium mb-2">Kampers</div>
                {isViewMode ? (
                  <div className="space-y-1">
                    {(table.selectedStudents || []).map(studentId => {
                      const student = students.find(s => s.id === studentId);
                      return student ? (
                        <div key={studentId} className="text-sm flex items-center justify-between p-2 bg-muted rounded">
                          <span>{student.firstName}</span>
                          <Badge variant="outline" className="text-xs">
                            {getLevelName(student.level)}
                          </Badge>
                        </div>
                      ) : null;
                    })}
                    {(!table.selectedStudents || table.selectedStudents.length === 0) && (
                      <span className="text-muted-foreground text-sm">Geen kampers geselecteerd</span>
                    )}
                  </div>
                ) : (
                  <div className="space-y-2">
                    {/* Selected students */}
                    <div className="space-y-1">
                      {(table.selectedStudents || []).map(studentId => {
                        const student = students.find(s => s.id === studentId);
                        return student ? (
                          <div key={studentId} className="flex items-center justify-between p-2 bg-muted rounded">
                            <span className="text-sm">{student.firstName}</span>
                            <Button
                              size="sm"
                              variant="ghost"
                              onClick={() => removeStudent(table.id, studentId)}
                              className="h-6 w-6 p-0"
                            >
                              <X className="h-3 w-3" />
                            </Button>
                          </div>
                        ) : null;
                      })}
                    </div>
                    
                    {/* Add student dropdown */}
                    <Popover>
                      <PopoverTrigger asChild>
                        <Button variant="outline" className="w-full justify-between">
                          Voeg kamper toe
                          <ChevronDown className="h-4 w-4" />
                        </Button>
                      </PopoverTrigger>
                      <PopoverContent className="w-64 p-3">
                        <div className="space-y-2 max-h-48 overflow-y-auto">
                          {getAvailableStudents(table.id).map(student => (
                            <div key={student.id} className="flex items-center space-x-2">
                              <Checkbox
                                id={`${table.id}-${student.id}`}
                                checked={(table.selectedStudents || []).includes(student.id)}
                                onCheckedChange={() => toggleStudent(table.id, student.id)}
                              />
                              <label 
                                htmlFor={`${table.id}-${student.id}`} 
                                className="text-sm flex-1 cursor-pointer"
                              >
                                {student.firstName} ({getLevelName(student.level)})
                              </label>
                            </div>
                          ))}
                          {getAvailableStudents(table.id).length === 0 && (
                            <div className="text-sm text-muted-foreground">
                              {(table.selectedLevels && table.selectedLevels.length > 0) 
                                ? "Geen beschikbare kampers voor geselecteerde niveaus"
                                : "Selecteer eerst een niveau om kampers te zien"
                              }
                            </div>
                          )}
                        </div>
                      </PopoverContent>
                    </Popover>
                  </div>
                )}
              </div>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}