import React from 'react';
import { Student } from '../App';
import { Card, CardContent, CardHeader, CardTitle } from './ui/card';
import { Button } from './ui/button';
import { Badge } from './ui/badge';

interface StudentSelectionProps {
  students: Student[];
  selectedLevel: number;
  onStudentSelect: (student: Student) => void;
}

export function StudentSelection({ students, selectedLevel, onStudentSelect }: StudentSelectionProps) {
  const getLevelName = (levelNumber: number) => {
    const levels = [
      { number: 1, name: "Ketelbinkie" },
      { number: 2, name: "Bemanning" },
      { number: 3, name: "A" },
      { number: 4, name: "B" }
    ];
    const level = levels.find(l => l.number === levelNumber);
    return level ? level.name : `Level ${levelNumber}`;
  };

  const filteredStudents = students.filter(student => student.level === selectedLevel);

  return (
    <div className="space-y-4">
      <div className="flex items-center gap-2">
        <span className="text-muted-foreground">Niveau:</span>
        <Badge variant="secondary">{getLevelName(selectedLevel)}</Badge>
      </div>

      {filteredStudents.length === 0 ? (
        <Card>
          <CardContent className="py-8">
            <div className="text-center text-muted-foreground">
              <p>Geen kampers gevonden voor niveau {getLevelName(selectedLevel)}</p>
              <p className="text-sm mt-2">Voeg eerst kampers toe via het kampers invoeren scherm.</p>
            </div>
          </CardContent>
        </Card>
      ) : (
        <div className="space-y-3">
          {filteredStudents.map(student => (
            <Card key={student.id} className="cursor-pointer hover:shadow-md transition-shadow">
              <CardHeader className="pb-3">
                <CardTitle className="text-lg">{student.firstName}</CardTitle>
              </CardHeader>
              <CardContent className="pt-0">
                <div className="flex items-center justify-between">
                  <Badge variant="outline">{getLevelName(student.level)}</Badge>
                  <Button 
                    onClick={() => onStudentSelect(student)}
                    size="sm"
                  >
                    Selecteer Kamper
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))}
        </div>
      )}
    </div>
  );
}