import React, { useState, useEffect } from 'react';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from './ui/select';
import { Trash2, Plus, Save, Check } from 'lucide-react';
import { Student } from '../App';

interface StudentManagementProps {
  students: Student[];
  onStudentsUpdate: (students: Student[]) => void;
  isOnline: boolean;
}

export const StudentManagement: React.FC<StudentManagementProps> = ({
  students,
  onStudentsUpdate,
  isOnline
}) => {
  const [localStudents, setLocalStudents] = useState<Student[]>(students);
  const [showSaveIndicator, setShowSaveIndicator] = useState(false);
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    setLocalStudents(students);
  }, [students]);

  const getLevelName = (level: number): string => {
    switch (level) {
      case 1: return 'Ketelbinkie';
      case 2: return 'Bemanning';
      case 3: return 'A';
      case 4: return 'B';
      default: return '';
    }
  };

  const handleAddStudent = () => {
    const newStudent: Student = {
      id: Date.now().toString(),
      firstName: '',
      level: 1
    };
    setLocalStudents([newStudent, ...localStudents]);
  };

  const handleUpdateStudent = (id: string, field: keyof Student, value: string | number) => {
    setLocalStudents(prev => prev.map(student => 
      student.id === id ? { ...student, [field]: value } : student
    ));
  };

  const handleDeleteStudent = (id: string) => {
    setLocalStudents(prev => prev.filter(student => student.id !== id));
  };

  const handleSave = async () => {
    setIsSaving(true);
    
    try {
      await onStudentsUpdate(localStudents);
      
      // Show success indicator
      setShowSaveIndicator(true);
      setTimeout(() => setShowSaveIndicator(false), 2000);
    } catch (error) {
      console.error('Error saving students:', error);
    } finally {
      setIsSaving(false);
    }
  };

  const SaveIndicator = () => (
    showSaveIndicator && (
      <div className="fixed bottom-4 right-4 bg-green-600 text-white px-3 py-2 rounded-lg shadow-lg flex items-center gap-2 z-50 animate-in slide-in-from-bottom duration-300">
        <Check className="h-4 w-4" />
        <span className="text-sm">Opgeslagen!</span>
      </div>
    )
  );

  return (
    <div className="space-y-4">
      {/* Top Section: Add Student and Save */}
      <div className="flex flex-col gap-4">
        <div className="flex justify-between items-center">
          <Button 
            onClick={handleAddStudent}
            className="bg-[#E60000] hover:bg-[#cc0000] text-white"
          >
            <Plus className="h-4 w-4 mr-2" />
            Kamper toevoegen
          </Button>
          
          <Button 
            onClick={handleSave}
            disabled={isSaving}
            className="bg-[#E60000] hover:bg-[#cc0000] text-white"
          >
            {isSaving ? (
              <>
                <div className="w-4 h-4 border-2 border-white/20 border-t-white rounded-full animate-spin mr-2" />
                Opslaan...
              </>
            ) : (
              <>
                <Save className="h-4 w-4 mr-2" />
                Opslaan
              </>
            )}
          </Button>
        </div>
      </div>

      {/* Students List */}
      <div className="space-y-3">
        {localStudents.map((student) => (
          <div key={student.id} className="bg-white p-4 rounded-lg border border-gray-200 shadow-sm">
            <div className="flex gap-3 items-center">
              <div className="flex-1">
                <Input
                  value={student.firstName}
                  onChange={(e) => handleUpdateStudent(student.id, 'firstName', e.target.value)}
                  placeholder="Voornaam"
                  className="w-full"
                />
              </div>
              
              <div className="flex-1">
                <Select
                  value={student.level.toString()}
                  onValueChange={(value) => handleUpdateStudent(student.id, 'level', parseInt(value))}
                >
                  <SelectTrigger className="w-full">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="1">Ketelbinkie</SelectItem>
                    <SelectItem value="2">Bemanning</SelectItem>
                    <SelectItem value="3">A</SelectItem>
                    <SelectItem value="4">B</SelectItem>
                  </SelectContent>
                </Select>
              </div>
              
              <Button
                onClick={() => handleDeleteStudent(student.id)}
                variant="destructive"
                size="sm"
              >
                <Trash2 className="h-4 w-4" />
              </Button>
            </div>
          </div>
        ))}
      </div>

      {localStudents.length === 0 && (
        <div className="text-center py-8 text-gray-500">
          <p>Nog geen kampers toegevoegd.</p>
          <p>Klik op "Kamper toevoegen" om te beginnen.</p>
        </div>
      )}

      {/* Bottom Save Button */}
      <div className="flex justify-center pt-4">
        <Button 
          onClick={handleSave}
          disabled={isSaving}
          className="bg-[#E60000] hover:bg-[#cc0000] text-white"
        >
          {isSaving ? (
            <>
              <div className="w-4 h-4 border-2 border-white/20 border-t-white rounded-full animate-spin mr-2" />
              Opslaan...
            </>
          ) : (
            <>
              <Save className="h-4 w-4 mr-2" />
              Opslaan
            </>
          )}
        </Button>
      </div>

      {/* Save Success Indicator */}
      <SaveIndicator />
    </div>
  );
};