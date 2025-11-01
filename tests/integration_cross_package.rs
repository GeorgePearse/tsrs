/// Integration tests for cross-package analysis functionality
/// Tests real Python package scenarios to validate Phase 1 & 2 implementation

use tsrs::CallGraphAnalyzer;

#[test]
fn test_scenario_1_simple_app_utils() {
    // Scenario 1: Simple app + utils pattern
    // Expected: app imports from utils, identifies dead code in both packages

    let mut analyzer = CallGraphAnalyzer::new();

    // Analyze utils package
    let utils_code = r#"
def validate_email(email):
    """Validate email format"""
    return "@" in email

def format_date(date_str):
    """Format date string"""
    return date_str.replace("-", "/")

def unused_helper():
    """This function is never imported or called"""
    return "dead code"

def parse_json(data):
    """Parse JSON data"""
    import json
    return json.loads(data)
"#;

    analyzer.analyze_source("utils", utils_code).expect("Failed to analyze utils");

    // Analyze app package
    let app_code = r#"
from utils import validate_email, format_date, parse_json

def register_user(email, date_joined):
    """Register a new user"""
    if validate_email(email):
        formatted = format_date(date_joined)
        return {"email": email, "joined": formatted}
    return None

def process_data(json_string):
    """Process JSON data"""
    data = parse_json(json_string)
    return data

def local_dead_code():
    """This is dead code - never called anywhere"""
    return "unused"

def test_main():
    """Entry point - test function"""
    user = register_user("test@example.com", "2024-01-15")
    print(user)
"#;

    analyzer.analyze_source("app", app_code).expect("Failed to analyze app");

    // Register imports manually since analyze_source doesn't track cross-package imports yet
    analyzer.add_import("app".to_string(), "validate_email".to_string(),
                       "utils".to_string(), "validate_email".to_string());
    analyzer.add_import("app".to_string(), "format_date".to_string(),
                       "utils".to_string(), "format_date".to_string());
    analyzer.add_import("app".to_string(), "parse_json".to_string(),
                       "utils".to_string(), "parse_json".to_string());

    let _reachable = analyzer.compute_reachable();

    // Find dead code
    let dead_code = analyzer.find_dead_code();
    let dead_names: Vec<String> = dead_code.iter().map(|(_, name)| name.clone()).collect();

    // Verify dead code detection
    assert!(dead_names.contains(&"unused_helper".to_string()), "unused_helper should be dead code");
    assert!(dead_names.contains(&"local_dead_code".to_string()), "local_dead_code should be dead code");

    // Verify live functions exist
    assert!(!dead_names.contains(&"test_main".to_string()), "test_main should not be dead (entry point)");
    assert!(!dead_names.contains(&"register_user".to_string()), "register_user should be live (called from test_main)");
    assert!(!dead_names.contains(&"validate_email".to_string()), "validate_email should be live (called from register_user)");
}

#[test]
fn test_scenario_2_multi_layer_shared_library() {
    // Scenario 2: Multi-layer shared library pattern
    // shared -> core, helpers (two packages depend on same shared library)

    let mut analyzer = CallGraphAnalyzer::new();

    // Analyze shared package
    let shared_code = r#"
def get_db_connection():
    """Get database connection"""
    return {"host": "localhost", "port": 5432}

def log_message(msg):
    """Log a message"""
    print(f"[LOG] {msg}")

def unused_shared_function():
    """This is never used by any package"""
    return "dead"

def get_config():
    """Get configuration"""
    return {"debug": True, "version": "1.0"}
"#;

    analyzer.analyze_source("shared", shared_code).expect("Failed to analyze shared");

    // Analyze core package
    let core_code = r#"
from shared import get_db_connection, log_message

def initialize_db():
    """Initialize database"""
    conn = get_db_connection()
    log_message(f"Connecting to {conn['host']}")
    return conn

def query_users():
    """Query users from database"""
    conn = initialize_db()
    log_message("Fetching users")
    return []

def unused_core_function():
    """Never called"""
    return "dead"

def test_query():
    """Test function - entry point"""
    return query_users()
"#;

    analyzer.analyze_source("core", core_code).expect("Failed to analyze core");

    // Analyze helpers package
    let helpers_code = r#"
from shared import log_message, get_config

def format_output(data):
    """Format output data"""
    log_message("Formatting output")
    config = get_config()
    return {"data": data, "debug": config["debug"]}

def validate_input(data):
    """Validate input data"""
    if not data:
        log_message("Invalid input")
        return False
    return True

def dead_helper():
    """Never used"""
    return "dead"

def test_format():
    """Test function - entry point"""
    return format_output({})

def test_validate():
    """Test function - entry point"""
    return validate_input("test")
"#;

    analyzer.analyze_source("helpers", helpers_code).expect("Failed to analyze helpers");

    // Register imports
    analyzer.add_import("core".to_string(), "get_db_connection".to_string(),
                       "shared".to_string(), "get_db_connection".to_string());
    analyzer.add_import("core".to_string(), "log_message".to_string(),
                       "shared".to_string(), "log_message".to_string());

    analyzer.add_import("helpers".to_string(), "log_message".to_string(),
                       "shared".to_string(), "log_message".to_string());
    analyzer.add_import("helpers".to_string(), "get_config".to_string(),
                       "shared".to_string(), "get_config".to_string());

    let dead_code = analyzer.find_dead_code();
    let dead_names: Vec<String> = dead_code.iter().map(|(_, name)| name.clone()).collect();

    // Verify dead code detection
    assert!(dead_names.contains(&"unused_shared_function".to_string()), "unused_shared_function should be dead");
    assert!(dead_names.contains(&"unused_core_function".to_string()), "unused_core_function should be dead");
    assert!(dead_names.contains(&"dead_helper".to_string()), "dead_helper should be dead");

    // Verify live functions
    assert!(!dead_names.contains(&"test_query".to_string()), "test_query should be live (entry point)");
    assert!(!dead_names.contains(&"test_format".to_string()), "test_format should be live (entry point)");
    assert!(!dead_names.contains(&"test_validate".to_string()), "test_validate should be live (entry point)");
    assert!(!dead_names.contains(&"query_users".to_string()), "query_users should be live (called from test_query)");
    assert!(!dead_names.contains(&"format_output".to_string()), "format_output should be live (called from test_format)");
    assert!(!dead_names.contains(&"validate_input".to_string()), "validate_input should be live (called from test_validate)");
}

#[test]
fn test_scenario_3_service_pattern() {
    // Scenario 3: Service pattern
    // service imports from shared

    let mut analyzer = CallGraphAnalyzer::new();

    // Analyze shared package
    let shared_code = r#"
def get_config():
    """Get configuration"""
    return {"debug": True, "version": "2.0"}

def log_message(msg):
    """Log a message"""
    print(f"[LOG] {msg}")

def unused_shared_function():
    """This is never used by any package"""
    return "dead"
"#;

    analyzer.analyze_source("shared", shared_code).expect("Failed to analyze shared");

    // Analyze service package
    let service_code = r#"
from shared import get_config, log_message

def initialize_service():
    """Initialize service"""
    config = get_config()
    log_message(f"Service initialized with version {config['version']}")
    return config

def start_service():
    """Start the service"""
    config = initialize_service()
    log_message("Service started")
    return True

def handle_request(request_data):
    """Handle incoming request"""
    log_message(f"Processing request: {request_data}")
    return {"status": "ok", "data": request_data}

def unused_service_function():
    """Never called"""
    return "dead"

def test_service():
    """Test function - entry point"""
    start_service()
    handle_request({"test": "data"})
"#;

    analyzer.analyze_source("service", service_code).expect("Failed to analyze service");

    // Register imports
    analyzer.add_import("service".to_string(), "get_config".to_string(),
                       "shared".to_string(), "get_config".to_string());
    analyzer.add_import("service".to_string(), "log_message".to_string(),
                       "shared".to_string(), "log_message".to_string());

    let dead_code = analyzer.find_dead_code();
    let dead_names: Vec<String> = dead_code.iter().map(|(_, name)| name.clone()).collect();

    // Verify dead code detection
    assert!(dead_names.contains(&"unused_shared_function".to_string()), "unused_shared_function should be dead");
    assert!(dead_names.contains(&"unused_service_function".to_string()), "unused_service_function should be dead");

    // Verify live functions
    assert!(!dead_names.contains(&"test_service".to_string()), "test_service should be live (entry point)");
    assert!(!dead_names.contains(&"start_service".to_string()), "start_service should be live (called from test_service)");
    assert!(!dead_names.contains(&"handle_request".to_string()), "handle_request should be live (called from test_service)");
    assert!(!dead_names.contains(&"initialize_service".to_string()), "initialize_service should be live (called from start_service)");
}

#[test]
fn test_imports_tracking_across_packages() {
    // Verify that imports are correctly tracked and resolved
    let mut analyzer = CallGraphAnalyzer::new();

    let package_a_code = r#"
def function_a():
    """Function in package A"""
    return "a"
"#;

    let package_b_code = r#"
from package_a import function_a as func_a_alias

def function_b():
    """Function in package B"""
    return func_a_alias()
"#;

    analyzer.analyze_source("package_a", package_a_code).expect("Failed to analyze package_a");
    analyzer.analyze_source("package_b", package_b_code).expect("Failed to analyze package_b");

    // Register import manually
    analyzer.add_import("package_b".to_string(), "func_a_alias".to_string(),
                       "package_a".to_string(), "function_a".to_string());

    // Verify import resolution
    let imports = analyzer.get_imports_for_package("package_b");
    assert_eq!(imports.len(), 1, "package_b should have 1 import");

    let (local_name, source_pkg, source_func) = &imports[0];
    assert_eq!(local_name, "func_a_alias", "local name should match");
    assert_eq!(source_pkg, "package_a", "source package should match");
    assert_eq!(source_func, "function_a", "source function should match");

    // Verify resolve_call works
    let resolved = analyzer.resolve_call("package_b", "func_a_alias");
    assert!(resolved.is_some(), "Should resolve func_a_alias to package_a function_a");
    let (pkg, func) = resolved.unwrap();
    assert_eq!(pkg, "package_a", "Should resolve to package_a");
    assert_eq!(func, "function_a", "Should resolve to function_a");
}
