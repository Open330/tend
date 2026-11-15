import XCTest
@testable import Tend

final class ServerOriginTests: XCTestCase {
    func testAcceptsBareHTTPSOrigins() throws {
        XCTAssertEqual(try ServerOrigin(parsing: "https://tend.example.com").display, "https://tend.example.com")
        XCTAssertEqual(try ServerOrigin(parsing: " tend.example.com/ ").display, "https://tend.example.com")
        XCTAssertEqual(try ServerOrigin(parsing: "HTTPS://Tend.Example.com:8443").display, "https://tend.example.com:8443")
    }

    func testPlainHTTPOnlyForLoopback() throws {
        XCTAssertEqual(try ServerOrigin(parsing: "http://127.0.0.1:7979").display, "http://127.0.0.1:7979")
        XCTAssertNoThrow(try ServerOrigin(parsing: "http://localhost:7878"))
        XCTAssertThrowsError(try ServerOrigin(parsing: "http://tend.example.com")) {
            XCTAssertEqual($0 as? ServerOrigin.Problem, .plainHTTP)
        }
        XCTAssertThrowsError(try ServerOrigin(parsing: "http://192.168.1.2:7878"))
    }

    func testRefusesAnythingButAnOrigin() {
        for input in [
            "https://user:pass@tend.example.com",
            "https://tend.example.com/needs",
            "https://tend.example.com/?token=x",
            "https://tend.example.com/#token=x",
        ] {
            XCTAssertThrowsError(try ServerOrigin(parsing: input), input) {
                XCTAssertEqual($0 as? ServerOrigin.Problem, .notBare, input)
            }
        }
        XCTAssertThrowsError(try ServerOrigin(parsing: "ftp://tend.example.com"))
        XCTAssertThrowsError(try ServerOrigin(parsing: ""))
    }

    func testEndpointsStayOnTheOrigin() throws {
        let origin = try ServerOrigin(parsing: "https://tend.example.com")
        XCTAssertEqual(origin.endpoint("/api/decisions").absoluteString, "https://tend.example.com/api/decisions")
    }
}
