import React, { useState, useEffect } from 'react';
import { Login } from './components/Login';
import { StudentManagement } from './components/StudentManagement';
import { LevelSelection } from './components/LevelSelection';
import { StudentSelection } from './components/StudentSelection';
import { ProgressRecord } from './components/ProgressRecord';
import { DaySelection } from './components/DaySelection';
import { BoatAllocation } from './components/BoatAllocation';
import { Button } from './components/ui/button';
import { Sheet, SheetContent, SheetTrigger, SheetHeader, SheetTitle, SheetDescription } from './components/ui/sheet';
import { ArrowLeft, Menu, LogOut, Wifi, WifiOff } from 'lucide-react';
import { studentsAPI, progressAPI, boatAllocationsAPI } from './utils/api';
import { toast } from 'sonner';
import { Toaster } from './components/ui/sonner';

export interface Student {
  id: string;
  firstName: string;
  level: number;
}

export interface ProgressEntry {
  skill: string;
  subSkills: string[];
}

export interface SkillCategory {
  category: string;
  evaluationType: 'standard' | 'overall'; // 'overall' for Algemene indruk
  skills: ProgressEntry[];
}

export interface DayRecord {
  skipperName: string;
  records: { [skillKey: string]: '✓' | '•' | '' | 'O' | 'T' | 'V' | 'G' };
}

export interface StudentProgress {
  studentId: string;
  days: DayRecord[];
  notes?: string;
}

export interface BoatTable {
  id: string;
  selectedLevels: string[];
  boatAndSkipperName: string;
  selectedStudents: string[];
}

export interface DayAllocation {
  day: string;
  tables: BoatTable[];
  isViewMode: boolean;
}

// Different skill matrices based on level, organized by categories
const getSkillsForLevel = (level: number): SkillCategory[] => {
  switch (level) {
    case 1: // Ketelbinkie - Easiest
      return [
        {
          category: "Schip in rust",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Zeilklaar maken grootzeil",
              subSkills: ["Vallen aanslaan", "Huikje"]
            },
            {
              skill: "Zeilklaar maken fok",
              subSkills: ["Fok hijsen"]
            },
            {
              skill: "Hijsen grootzeil",
              subSkills: ["Volgorde", "Hijsen", "Afwerking"]
            },
            {
              skill: "Hijsen fok",
              subSkills: []
            },            
            {
              skill: "Strijken grootzeil",
              subSkills: ["Volgorde", "Snelheid", "Zeil opdoeken"]
            },
            {
              skill: "Strijken fok",
              subSkills: []
            },            
            {
              skill: "Afmeren Box Rufus",
              subSkills: ["Volgorde", "Afwerking"]
            },
            {
              skill: "Nachtklaar maken",
              subSkills: ["Huikje", "Schoten/ vallen", "Breien", "Poetsen"]
            },
            {
              skill: "Verhalen schip langs de wal",
              subSkills: []
            },
            {
              skill: "Verhalen schip 180º",
              subSkills: []
            },
            {
              skill: "Bomen",
              subSkills: []
            },
            {
              skill: "Knopen",
              subSkills: []
            },
            {
              skill: "Peddelen",
              subSkills: []
            }
          ]
        },
        {
          category: "Basisvaardigheden",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Sturen",
              subSkills: []
            },
            {
              skill: "Koersen",
              subSkills: ["Windrichting", "Namen", "Oploeven/ afvallen", "Grenskoersen VDW", "Grenskoersen ADW"]
            },
            {
              skill: "Zeilstanden",
              subSkills: ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]
            },    
            {
              skill: "Overstag",
              subSkills: ["ADW", "Commando's", "Sturen"]
            },            
            {
              skill: "Dwarspeiling",
              subSkills: []
            },
            {
              skill: "Gijpen",
              subSkills: ["VDW", "Commando's", "Werking roer"]
            }
          ]
        },
        {
          category: "Bijzondere verrichtingen",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Eilandje ronden",
              subSkills: []
            },
            {
              skill: "Achtje varen",
              subSkills: []
            }
          ]
        },
        {
          category: "Algemene indruk",
          evaluationType: 'overall',
          skills: [
            {
              skill: "Algemene indruk",
              subSkills: ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"]
            }
          ]
        }
      ];
    
    case 2: // Bemanning - More difficult
      return [
        {
          category: "Schip in rust",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Zeilklaar maken grootzeil",
              subSkills: ["Vallen aanslaan", "Huikje"]
            },
            {
              skill: "Zeilklaar maken fok",
              subSkills: ["Fok hijsen"]
            },
            {
              skill: "Hijsen grootzeil",
              subSkills: ["Volgorde", "Hijsen", "Afwerking"]
            },
            {
              skill: "Hijsen fok",
              subSkills: []
            },            
            {
              skill: "Strijken grootzeil",
              subSkills: ["Volgorde", "Snelheid", "Zeil opdoeken"]
            },
            {
              skill: "Strijken fok",
              subSkills: []
            },            
            {
              skill: "Afmeren Box Rufus",
              subSkills: ["Volgorde", "Afwerking"]
            },
            {
              skill: "Nachtklaar maken",
              subSkills: ["Huikje/ schoten/ vallen", "Breien/ poetsen"]
            },
            {
              skill: "Verhalen schip langs de wal",
              subSkills: []
            },
            {
              skill: "Verhalen schip 180º",
              subSkills: []
            },
            {
              skill: "Bomen",
              subSkills: []
            },
            {
              skill: "Knopen",
              subSkills: []
            },
            {
              skill: "Peddelen",
              subSkills: []
            }
          ]
        },
       {
          category: "Basisvaardigheden",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Sturen",
              subSkills: []
            },
            {
              skill: "Koersen",
              subSkills: ["Windrichting/ namen", "Oploeven/ afvallen", "Grenskoersen ADW/ VDW"]
            },
            {
              skill: "Zeilstanden",
              subSkills: ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]
            },    
            {
              skill: "Overstag",
              subSkills: ["ADW", "Commando's", "Sturen"]
            },            
            {
              skill: "Dwarspeiling",
              subSkills: []
            },
            {
              skill: "Gijpen",
              subSkills: ["VDW", "Commando's", "Werking roer"]
            },
            {
              skill: "BPR",
              subSkills: []
            }
          ]
        },
        {
          category: "Bijzondere verrichtingen",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Aanleggen hogerwal",
              subSkills: ["ADW lijnen", "Dwarspeiling", "Snelheid regelen", "Wegdraaien", "Organisatie"]
            },
            {
              skill: "Afvaren hogerwal",
              subSkills: ["Roer deinzend", "Afduwrichting", "Fok", "Organisatie"]
            },
            {
              skill: "MOB",
              subSkills: ["Commando's", "Parcours", "Snelheid opl/ alv", "Dwarspeiling", "Snelheid regelen", "Organisatie"]
            },            
            {
              skill: "Stormronde/ halzen",
              subSkills: []
            }
          ]
        },
        {
          category: "Algemene indruk",
          evaluationType: 'overall',
          skills: [
            {
              skill: "Algemene indruk",
              subSkills: ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"]
            }
          ]
        }
      ];
    
    case 3: // A - Even more difficult
      return [
        {
          category: "Schip in rust",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Zeilklaar maken grootzeil",
              subSkills: []
            },
            {
              skill: "Hijsen grootzeil",
              subSkills: []
            },
            {
              skill: "Hijsen fok",
              subSkills: []
            },            
            {
              skill: "Strijken grootzeil",
              subSkills: []
            },
            {
              skill: "Strijken fok",
              subSkills: []
            },            
            {
              skill: "Afmeren",
              subSkills: []
            },
            {
              skill: "Nachtklaar maken",
              subSkills: []
            },
            {
              skill: "Verhalen schip",
              subSkills: []
            },
            {
              skill: "Bomen",
              subSkills: []
            },
            {
              skill: "Knopen",
              subSkills: []
            },
            {
              skill: "Reven",
              subSkills: []
            }
          ]
        },
       {
          category: "Basisvaardigheden",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Sturen",
              subSkills: []
            },
            {
              skill: "Koersen",
              subSkills: ["Windrichting/ namen", "Oploeven/ afvallen", "Grenskoersen ADW/ VDW"]
            },
            {
              skill: "Zeilstanden",
              subSkills: ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]
            },    
            {
              skill: "Overstag",
              subSkills: []
            },            
            {
              skill: "Dwarspeiling",
              subSkills: []
            },
            {
              skill: "Gijpen",
              subSkills: []
            },
            {
              skill: "BPR",
              subSkills: []
            },
            {
              skill: "Knopen",
              subSkills: []
            }            
          ]
        },
        {
          category: "Bijzondere verrichtingen",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Aanleggen hogerwal",
              subSkills: ["Dwarspeiling meth.", "Opdraai meth.", "Opschieter meth.", "ADW lijnen", "Dwarspeiling", "Snelheid regelen", "Wegdraaien", "Organisatie"]
            },
            {
              skill: "Afvaren hogerwal",
              subSkills: ["Roer deinzend", "Afduwrichting", "Fok", "Organisatie"]
            },
            {
              skill: "MOB",
              subSkills: ["Commando's", "Parcours", "Snelheid opl/ alv", "Dwarspeiling", "Snelheid regelen", "Organisatie"]
            },            
            {
              skill: "Varend hijsen/ strijken",
              subSkills: ["Organisatie", "Koers", "Kraanlijn"]
            },
            {
              skill: "Kruisen nauw water",
              subSkills: ["Korte slag", "Lange slag", "BPR", "Hoogtelopen"]
            },
            {
              skill: "Mast strijken",
              subSkills: []
            },
            {
              skill: "Mast zetten",
              subSkills: []
            }
          ]
        },               
        {
          category: "Algemene indruk",
          evaluationType: 'overall',
          skills: [
            {
              skill: "Algemene indruk",
              subSkills: ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"]
            }
          ]
        }
      ];
    
    case 4: // B - Most difficult
      return [
        {
          category: "Schip in rust",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Zeilklaar maken grootzeil",
              subSkills: []
            },
            {
              skill: "Hijsen grootzeil",
              subSkills: []
            },
            {
              skill: "Hijsen fok",
              subSkills: []
            },            
            {
              skill: "Strijken grootzeil",
              subSkills: []
            },
            {
              skill: "Strijken fok",
              subSkills: []
            },            
            {
              skill: "Afmeren",
              subSkills: []
            },
            {
              skill: "Nachtklaar maken",
              subSkills: []
            },
            {
              skill: "Verhalen schip",
              subSkills: []
            },
            {
              skill: "Bomen",
              subSkills: []
            },
            {
              skill: "Knopen",
              subSkills: []
            },
            {
              skill: "Reven",
              subSkills: []
            }
          ]
        },
       {
          category: "Basisvaardigheden",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Sturen",
              subSkills: []
            },
            {
              skill: "Koersen",
              subSkills: ["Windrichting/ namen", "Oploeven/ afvallen", "Grenskoersen ADW/ VDW"]
            },
            {
              skill: "Zeilstanden",
              subSkills: ["Op koers", "Bediening fok", "Zeilsturing opl/afv"]
            },    
            {
              skill: "Overstag",
              subSkills: []
            },            
            {
              skill: "Dwarspeiling",
              subSkills: []
            },
            {
              skill: "Gijpen",
              subSkills: []
            },
            {
              skill: "BPR",
              subSkills: []
            },
            {
              skill: "Knopen",
              subSkills: []
            }            
          ]
        },
        {
          category: "Bijzondere verrichtingen",
          evaluationType: 'standard',
          skills: [
            {
              skill: "Aanleggen hogerwal",
              subSkills: ["Dwarspeiling meth.", "Opdraai meth.", "Opschieter meth.", "ADW lijnen", "Dwarspeiling", "Snelheid regelen", "Wegdraaien", "Organisatie"]
            },
            {
              skill: "Afvaren hogerwal",
              subSkills: ["Roer deinzend", "Afduwrichting", "Fok", "Organisatie"]
            },
            {
              skill: "MOB",
              subSkills: ["Commando's", "Parcours", "Snelheid opl/ alv", "Dwarspeiling", "Snelheid regelen", "Organisatie"]
            },
            {
              skill: "Kruisen nauw water",
              subSkills: ["Korte slag", "Lange slag", "BPR", "Hoogtelopen"]
            },
            {
              skill: "Varend reven",
              subSkills: []
            },
            {
              skill: "Loskomen",
              subSkills: []
            },
            {
              skill: "Aanleggen lagerwal",
              subSkills: ["Parcours", "Organisatie", "Inzicht", "Strijken"]
            },            
            {
              skill: "Afvaren lagerwal",
              subSkills: ["Parcours", "Organisatie", "Inzicht", "Afduwen", "Hijsen"]
            },   
            {
              skill: "Ankeren",
              subSkills: ["Parcours", "Organisatie", "Deinzen", "Ankermaat"]
            },   
            {
              skill: "Ankerop gaan",
              subSkills: ["Organisatie", "Inzicht", "Ankermaat"]
            }               
          ]
        },        
        {
          category: "Algemene indruk",
          evaluationType: 'overall',
          skills: [
            {
              skill: "Algemene indruk",
              subSkills: ["Inzicht algemeen", "Inzicht bijzondere verrichtingen", "Organisatie", "Inzet", "Teamplayer"]
            }
          ]
        }
      ];
    
    default:
      return [];
  }
};

const weekDays = [
  'Zaterdag', 'Zondag', 'Maandag', 'Dinsdag', 'Woensdag'
];

type Screen = 'management' | 'levelSelection' | 'studentSelection' | 'progressRecord' | 'daySelection' | 'boatAllocation';

export default function App() {
  const [isAuthenticated, setIsAuthenticated] = useState(false);
  const [currentScreen, setCurrentScreen] = useState<Screen>('levelSelection');
  const [students, setStudents] = useState<Student[]>([]);
  const [selectedLevel, setSelectedLevel] = useState<number | null>(null);
  const [selectedStudent, setSelectedStudent] = useState<Student | null>(null);
  const [selectedDay, setSelectedDay] = useState<string | null>(null);
  const [progressData, setProgressData] = useState<StudentProgress[]>([]);
  const [boatAllocations, setBoatAllocations] = useState<DayAllocation[]>([]);
  const [isMenuOpen, setIsMenuOpen] = useState(false);
  const [isOnline, setIsOnline] = useState(navigator.onLine);
  const [isLoading, setIsLoading] = useState(true);

  // Monitor online status
  useEffect(() => {
    const handleOnline = () => setIsOnline(true);
    const handleOffline = () => setIsOnline(false);
    window.addEventListener('online', handleOnline);
    window.addEventListener('offline', handleOffline);
    return () => {
      window.removeEventListener('online', handleOnline);
      window.removeEventListener('offline', handleOffline);
    };
  }, []);

  // Check authentication on mount via server session
  useEffect(() => {
    fetch('/api/auth/me', { credentials: 'same-origin' })
      .then(r => { if (r.ok) setIsAuthenticated(true); })
      .catch(() => {})
      .finally(() => setIsLoading(false));
  }, []);

  // Load data from server when authenticated
  useEffect(() => {
    if (!isAuthenticated) return;

    const loadData = async () => {
      try {
        const [studentsData, progressDataResponse, boatAllocationsData] = await Promise.all([
          studentsAPI.getAll(),
          progressAPI.getAll(),
          boatAllocationsAPI.getAll(),
        ]);
        setStudents(studentsData);
        setProgressData(progressDataResponse);
        setBoatAllocations(boatAllocationsData);
      } catch (error) {
        console.error('Error loading data:', error);
        toast.error('Fout bij het laden van gegevens.');
      }
    };

    loadData();
  }, [isAuthenticated]);

  const handleLogin = () => {
    setIsAuthenticated(true);
  };

  const handleLogout = async () => {
    await fetch('/api/auth/logout', { method: 'POST', credentials: 'same-origin' }).catch(() => {});
    setIsAuthenticated(false);
    setCurrentScreen('levelSelection');
    setIsMenuOpen(false);
    setStudents([]);
    setProgressData([]);
    setBoatAllocations([]);
  };

  const handleBack = () => {
    switch (currentScreen) {
      case 'levelSelection':
        setCurrentScreen('levelSelection');
        break;
      case 'studentSelection':
        setCurrentScreen('levelSelection');
        setSelectedLevel(null);
        break;
      case 'progressRecord':
        setCurrentScreen('studentSelection');
        setSelectedStudent(null);
        break;
      case 'boatAllocation':
        setCurrentScreen('daySelection');
        setSelectedDay(null);
        break;
      default:
        break;
    }
  };

  const handleLevelSelect = (level: number) => {
    setSelectedLevel(level);
    setCurrentScreen('studentSelection');
  };

  const handleStudentSelect = (student: Student) => {
    setSelectedStudent(student);
    setCurrentScreen('progressRecord');
  };

  const handleDaySelect = (day: string) => {
    setSelectedDay(day);
    setCurrentScreen('boatAllocation');
  };

  const handleStudentsUpdate = async (updatedStudents: Student[]) => {
    setStudents(updatedStudents);
    try {
      await studentsAPI.save(updatedStudents);
    } catch (error) {
      console.error('Error saving students:', error);
      toast.error('Fout bij het opslaan van kampers.');
    }
  };

  const handleProgressUpdate = async (updatedProgress: StudentProgress) => {
    setProgressData(prev => {
      const existingIndex = prev.findIndex(p => p.studentId === updatedProgress.studentId);
      if (existingIndex >= 0) {
        const updated = [...prev];
        updated[existingIndex] = updatedProgress;
        return updated;
      }
      return [...prev, updatedProgress];
    });
    try {
      await progressAPI.save(updatedProgress);
    } catch (error) {
      console.error('Error saving progress:', error);
      toast.error('Fout bij het opslaan van evaluatie.');
    }
  };

  const handleBoatAllocationUpdate = async (updatedAllocation: DayAllocation) => {
    setBoatAllocations(prev => {
      const existingIndex = prev.findIndex(a => a.day === updatedAllocation.day);
      if (existingIndex >= 0) {
        const updated = [...prev];
        updated[existingIndex] = updatedAllocation;
        return updated;
      }
      return [...prev, updatedAllocation];
    });
    try {
      await boatAllocationsAPI.save(updatedAllocation);
      toast.success('Bootindeling opgeslagen!');
    } catch (error) {
      console.error('Error saving boat allocation:', error);
      toast.error('Fout bij het opslaan van bootindeling.');
    }
  };

  const navigateToScreen = (screen: Screen) => {
    setCurrentScreen(screen);
    setIsMenuOpen(false);
  };

  const getScreenTitle = () => {
    switch (currentScreen) {
      case 'management':
        return 'Kampers Invoeren';
      case 'levelSelection':
        return 'Niveau Selecteren';
      case 'studentSelection':
        return 'Kamper Selecteren';
      case 'progressRecord':
        return 'Evaluatiekaart';
      case 'daySelection':
        return 'Bootindeling';
      case 'boatAllocation':
        return `Bootindeling - ${selectedDay}`;
      default:
        return '';
    }
  };

  // Helper function to sort students alphabetically
  const sortStudentsAlphabetically = (students: Student[]): Student[] => {
    return [...students].sort((a, b) => a.firstName.localeCompare(b.firstName));
  };

  const renderScreen = () => {
    if (isLoading) {
      return (
        <div className="flex items-center justify-center py-8">
          <div className="text-center">
            <div className="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin mx-auto mb-4"></div>
            <p>Gegevens laden...</p>
          </div>
        </div>
      );
    }

    switch (currentScreen) {
      case 'management':
        return (
          <StudentManagement
            students={students}
            onStudentsUpdate={handleStudentsUpdate}
            isOnline={isOnline}
          />
        );
      case 'levelSelection':
        return (
          <LevelSelection
            onLevelSelect={handleLevelSelect}
          />
        );
      case 'studentSelection':
        return (
          <StudentSelection
            students={sortStudentsAlphabetically(students)}
            selectedLevel={selectedLevel!}
            onStudentSelect={handleStudentSelect}
          />
        );
      case 'progressRecord':
        return (
          <ProgressRecord
            student={selectedStudent!}
            skillCategories={getSkillsForLevel(selectedStudent?.level || 1)}
            progressData={progressData.find(p => p.studentId === selectedStudent?.id)}
            onProgressUpdate={handleProgressUpdate}
            isOnline={isOnline}
          />
        );
      case 'daySelection':
        return (
          <DaySelection
            days={weekDays}
            onDaySelect={handleDaySelect}
          />
        );
      case 'boatAllocation':
        return (
          <BoatAllocation
            day={selectedDay!}
            students={sortStudentsAlphabetically(students)}
            existingAllocation={boatAllocations.find(a => a.day === selectedDay)}
            onAllocationUpdate={handleBoatAllocationUpdate}
          />
        );
      default:
        return null;
    }
  };

  // Show login screen if not authenticated (or still loading)
  if (!isAuthenticated) {
    if (isLoading) {
      return (
        <div className="min-h-screen flex items-center justify-center">
          <div className="w-8 h-8 border-4 border-primary/20 border-t-primary rounded-full animate-spin" />
        </div>
      );
    }
    return (
      <>
        <Toaster />
        <Login onLogin={handleLogin} />
      </>
    );
  }

  return (
    <>
    <Toaster />
    <div className="min-h-screen bg-background">
      {/* Mobile Header */}
      <div className="sticky top-0 z-50 bg-background border-b border-border px-4 py-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <Sheet open={isMenuOpen} onOpenChange={setIsMenuOpen}>
              <SheetTrigger asChild>
                <Button variant="ghost" size="sm" className="p-2">
                  <Menu className="h-5 w-5" />
                </Button>
              </SheetTrigger>
              <SheetContent side="left" className="w-72 p-0">
                <SheetHeader className="p-6 pb-4">
                  <SheetTitle>Menu</SheetTitle>
                  <SheetDescription>
                    Navigeer door de verschillende onderdelen van de app
                  </SheetDescription>
                </SheetHeader>
                <div className="px-6 pb-6">
                  <div className="space-y-3">
                    <Button 
                      variant="ghost" 
                      className="w-full justify-start"
                      onClick={() => navigateToScreen('management')}
                    >
                      Kampers invoeren
                    </Button>
                    <Button 
                      variant="ghost" 
                      className="w-full justify-start"
                      onClick={() => navigateToScreen('levelSelection')}
                    >
                      Evaluatiekaarten
                    </Button>
                    <Button 
                      variant="ghost" 
                      className="w-full justify-start"
                      onClick={() => navigateToScreen('daySelection')}
                    >
                      Bootindeling
                    </Button>
                    <div className="pt-4 border-t space-y-3">
                      <a href="/optimizer" className="block">
                        <Button variant="ghost" className="w-full justify-start">
                          Rooster Optimizer
                        </Button>
                      </a>
                      <Button
                        variant="ghost"
                        className="w-full justify-start text-destructive hover:text-destructive"
                        onClick={handleLogout}
                      >
                        <LogOut className="h-4 w-4 mr-2" />
                        Uitloggen
                      </Button>
                    </div>
                  </div>
                </div>
              </SheetContent>
            </Sheet>
            
            {currentScreen !== 'management' && currentScreen !== 'levelSelection' && currentScreen !== 'daySelection' && (
              <Button
                variant="ghost"
                size="sm"
                onClick={handleBack}
                className="p-2"
              >
                <ArrowLeft className="h-5 w-5" />
              </Button>
            )}
          </div>
          
          <h1 className="text-lg">{getScreenTitle()}</h1>
          
          {/* Connection status indicator */}
          <div className="flex items-center gap-2">
            {isOnline ? (
              <Wifi className="h-4 w-4 text-green-600" />
            ) : (
              <WifiOff className="h-4 w-4 text-red-600" />
            )}
          </div>
        </div>
      </div>

      {/* Offline banner */}
      {!isOnline && (
        <div className="bg-yellow-100 border-b border-yellow-200 px-4 py-2 text-center text-sm text-yellow-800">
          Offline modus - wijzigingen worden lokaal opgeslagen
        </div>
      )}

      {/* Main Content */}
      <div className="p-4">
        {renderScreen()}
      </div>
    </div>
    </>
  );
}