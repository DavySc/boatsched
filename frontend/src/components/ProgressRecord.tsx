import React, { useState, useEffect } from 'react';
import { Button } from './ui/button';
import { Input } from './ui/input';
import { Textarea } from './ui/textarea';
import { Check, Save } from 'lucide-react';
import { Student, SkillCategory, StudentProgress, DayRecord } from '../App';

interface ProgressRecordProps {
  student: Student;
  skillCategories: SkillCategory[];
  progressData?: StudentProgress;
  onProgressUpdate: (progress: StudentProgress) => void;
  isOnline: boolean;
}

export const ProgressRecord: React.FC<ProgressRecordProps> = ({
  student,
  skillCategories,
  progressData,
  onProgressUpdate,
  isOnline
}) => {
  const [days, setDays] = useState<DayRecord[]>([]);
  const [notes, setNotes] = useState('');
  const [showSaveIndicator, setShowSaveIndicator] = useState(false);
  const [isSaving, setIsSaving] = useState(false);

  // Initialize days array with 5 empty days if no progress data exists
  useEffect(() => {
    if (progressData) {
      setDays(progressData.days || []);
      setNotes(progressData.notes || '');
    } else {
      // Create 5 empty days with all skills initialized
      const emptyDays: DayRecord[] = Array.from({ length: 5 }, (_, index) => {
        const records: { [skillKey: string]: '✓' | '•' | '' | 'O' | 'T' | 'V' | 'G' } = {};
        
        skillCategories.forEach(category => {
          category.skills.forEach(skill => {
            // Always track the main skill
            records[skill.skill] = '';
            
            // Track sub-skills if they exist
            skill.subSkills.forEach(subSkill => {
              records[`${skill.skill}-${subSkill}`] = '';
            });
          });
        });

        return {
          skipperName: '',
          records
        };
      });
      setDays(emptyDays);
      setNotes('');
    }
  }, [progressData, skillCategories]);

  const handleSkipperNameChange = (dayIndex: number, name: string) => {
    const updatedDays = [...days];
    updatedDays[dayIndex] = { ...updatedDays[dayIndex], skipperName: name };
    setDays(updatedDays);
  };

  const handleCellClick = (dayIndex: number, skillKey: string, category: SkillCategory) => {
    const updatedDays = [...days];
    const currentValue = updatedDays[dayIndex].records[skillKey];
    
    let nextValue: '✓' | '•' | '' | 'O' | 'T' | 'V' | 'G';
    
    if (category.evaluationType === 'overall') {
      // For "Algemene indruk" - cycle through O, T, V, G, empty
      switch (currentValue) {
        case 'O': nextValue = 'T'; break;
        case 'T': nextValue = 'V'; break;
        case 'V': nextValue = 'G'; break;
        case 'G': nextValue = ''; break;
        default: nextValue = 'O'; break;
      }
    } else {
      // For standard skills - cycle through •, ✓, empty (dot first, then checkmark)
      switch (currentValue) {
        case '•': nextValue = '✓'; break;
        case '✓': nextValue = ''; break;
        default: nextValue = '•'; break;
      }
    }
    
    updatedDays[dayIndex].records[skillKey] = nextValue;
    setDays(updatedDays);
  };

  const handleSave = async () => {
    setIsSaving(true);
    
    const progressToSave: StudentProgress = {
      studentId: student.id,
      days,
      notes
    };

    try {
      await onProgressUpdate(progressToSave);
      
      // Show success indicator
      setShowSaveIndicator(true);
      setTimeout(() => setShowSaveIndicator(false), 2000);
    } catch (error) {
      console.error('Error saving progress:', error);
    } finally {
      setIsSaving(false);
    }
  };

  const getCellBackgroundColor = (value: string, category: SkillCategory): string => {
    if (category.evaluationType === 'overall') {
      // For "Algemene indruk" - colored backgrounds
      switch (value) {
        case 'O': return 'bg-red-100'; // Onvoldoende
        case 'T': return 'bg-yellow-100'; // Twijfelachtig
        case 'V': return 'bg-green-100'; // Voldoende
        case 'G': return 'bg-blue-100'; // Goed
        default: return 'bg-transparent';
      }
    } else {
      // For standard skills - colored backgrounds
      switch (value) {
        case '•': return 'bg-red-100'; // Geoefend
        case '✓': return 'bg-green-100'; // Beheerst
        default: return 'bg-transparent';
      }
    }
  };

  const renderCellContent = (value: string, category: SkillCategory) => {
    if (category.evaluationType === 'overall') {
      // For "Algemene indruk" - show black letters on colored backgrounds
      switch (value) {
        case 'O': return <span className="text-black text-base font-bold">O</span>;
        case 'T': return <span className="text-black text-base font-bold">T</span>;
        case 'V': return <span className="text-black text-base font-bold">V</span>;
        case 'G': return <span className="text-black text-base font-bold">G</span>;
        default: return '';
      }
    } else {
      // For standard skills - show black symbols on colored backgrounds
      switch (value) {
        case '•': return <span className="text-black text-base font-bold">•</span>;
        case '✓': return <span className="text-black text-base font-bold">✓</span>;
        default: return '';
      }
    }
  };

  const SaveButton = ({ className = '' }: { className?: string }) => (
    <Button 
      onClick={handleSave}
      disabled={isSaving}
      className={`bg-[#E60000] hover:bg-[#cc0000] text-white ${className}`}
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
  );

  const SaveIndicator = () => (
    showSaveIndicator && (
      <div className="fixed bottom-4 right-4 bg-green-600 text-white px-3 py-2 rounded-lg shadow-lg flex items-center gap-2 z-50 animate-in slide-in-from-bottom duration-300">
        <Check className="h-4 w-4" />
        <span className="text-sm">Opgeslagen!</span>
      </div>
    )
  );

  return (
    <div className="space-y-6">
      {/* Top Save Button */}
      <div className="flex justify-between items-center">
        <h2 className="text-xl">{student.firstName}</h2>
        <SaveButton />
      </div>

      {/* Evaluation Table */}
      <div className="overflow-x-auto">
        <table className="w-full border-collapse border border-gray-300">
          {/* Table Header */}
          <thead>
            <tr>
              <th className="bg-[#E60000] text-white p-3 border border-gray-300 text-left sticky left-0 z-20 min-w-[200px]">
                Vaardigheden
              </th>
              {[1, 2, 3, 4, 5].map(day => (
                <th key={day} className="bg-[#FFB8B8] p-3 border border-gray-300 text-center min-w-[100px]">
                  Dag {day}
                </th>
              ))}
            </tr>
          </thead>
          
          <tbody>
            {/* Skipper Names Row */}
            <tr>
              <td className="bg-gray-100 p-3 border border-gray-300 sticky left-0 z-10 text-sm">
                <strong>Schipper</strong>
              </td>
              {days.map((day, dayIndex) => (
                <td key={dayIndex} className="p-2 border border-gray-300">
                  <Input
                    value={day.skipperName}
                    onChange={(e) => handleSkipperNameChange(dayIndex, e.target.value)}
                    placeholder="Naam"
                    className="text-xs h-8 border-gray-300"
                  />
                </td>
              ))}
            </tr>

            {/* Skills by Category */}
            {skillCategories.map((category, categoryIndex) => (
              <React.Fragment key={categoryIndex}>
                {/* Category Header */}
                <tr>
                  <td className="bg-[#E60000] text-white p-3 border border-gray-300 sticky left-0 z-10 text-sm font-bold">
                    {category.category}
                  </td>
                  {[1, 2, 3, 4, 5].map(day => (
                    <td key={day} className="bg-gray-50 border border-gray-300"></td>
                  ))}
                </tr>

                {/* Skills in Category */}
                {category.skills.map((skill, skillIndex) => (
                  <React.Fragment key={skillIndex}>
                    {/* Main Skill Row */}
                    <tr>
                      <td className="bg-gray-200 p-3 border border-gray-300 sticky left-0 z-10 text-sm font-medium">
                        {skill.skill}
                      </td>
                      {days.map((day, dayIndex) => {
                        const cellValue = day.records[skill.skill] || '';
                        const backgroundColorClass = getCellBackgroundColor(cellValue, category);
                        return (
                          <td key={dayIndex} className={`border border-gray-300 p-0 ${backgroundColorClass}`}>
                            <button
                              onClick={() => handleCellClick(dayIndex, skill.skill, category)}
                              className="w-full h-12 flex items-center justify-center hover:brightness-110 transition-all border-0 bg-transparent"
                            >
                              {renderCellContent(cellValue, category)}
                            </button>
                          </td>
                        );
                      })}
                    </tr>

                    {/* Sub-skills Rows */}
                    {skill.subSkills.map((subSkill, subIndex) => (
                      <tr key={subIndex}>
                        <td className="bg-gray-100 p-3 border border-gray-300 sticky left-0 z-10 text-xs pl-8">
                          {subSkill}
                        </td>
                        {days.map((day, dayIndex) => {
                          const cellValue = day.records[`${skill.skill}-${subSkill}`] || '';
                          const backgroundColorClass = getCellBackgroundColor(cellValue, category);
                          return (
                            <td key={dayIndex} className={`border border-gray-300 p-0 ${backgroundColorClass}`}>
                              <button
                                onClick={() => handleCellClick(dayIndex, `${skill.skill}-${subSkill}`, category)}
                                className="w-full h-10 flex items-center justify-center hover:brightness-110 transition-all border-0 bg-transparent"
                              >
                                {renderCellContent(cellValue, category)}
                              </button>
                            </td>
                          );
                        })}
                      </tr>
                    ))}
                  </React.Fragment>
                ))}
              </React.Fragment>
            ))}
          </tbody>
        </table>
      </div>

      {/* Notes Section */}
      <div className="mt-6 space-y-2">
        <label className="text-sm">Opmerkingen:</label>
        <Textarea
          value={notes}
          onChange={(e) => setNotes(e.target.value)}
          placeholder="Voeg hier opmerkingen toe..."
          className="w-full border-gray-300"
          rows={4}
        />
      </div>

      {/* Bottom Save Button */}
      <div className="mt-6 flex justify-center">
        <SaveButton />
      </div>

      {/* Legend */}
      <div className="bg-gray-50 p-4 rounded-lg border border-gray-200">
        <h3 className="mb-2 text-sm font-medium">Legenda:</h3>
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-sm">
          <div>
            <p><strong>Standaard vaardigheden:</strong></p>
            <p className="flex items-center gap-2">
              <span className="bg-red-500 text-white px-2 py-1 rounded text-base font-bold">•</span>
              = Geoefend (eerst)
            </p>
            <p className="flex items-center gap-2">
              <span className="bg-green-500 text-white px-2 py-1 rounded text-base font-bold">✓</span>
              = Beheerst (tweede)
            </p>
          </div>
          <div>
            <p><strong>Algemene indruk:</strong></p>
            <p className="flex items-center gap-2">
              <span className="bg-red-500 text-white px-2 py-1 rounded text-base font-bold">O</span>
              = Onvoldoende
            </p>
            <p className="flex items-center gap-2">
              <span className="bg-yellow-500 text-white px-2 py-1 rounded text-base font-bold">T</span>
              = Twijfelachtig
            </p>
            <p className="flex items-center gap-2">
              <span className="bg-green-500 text-white px-2 py-1 rounded text-base font-bold">V</span>
              = Voldoende
            </p>
            <p className="flex items-center gap-2">
              <span className="bg-blue-500 text-white px-2 py-1 rounded text-base font-bold">G</span>
              = Goed
            </p>
          </div>
        </div>
      </div>

      {/* Save Success Indicator */}
      <SaveIndicator />
    </div>
  );
};