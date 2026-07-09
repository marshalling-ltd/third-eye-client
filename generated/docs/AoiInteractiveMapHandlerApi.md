# \AoiInteractiveMapHandlerApi

All URIs are relative to *http://127.0.0.1:8070*

Method | HTTP request | Description
------------- | ------------- | -------------
[**get_aoi_interactive_map_aggregated_results_handler**](AoiInteractiveMapHandlerApi.md#get_aoi_interactive_map_aggregated_results_handler) | **GET** /api/v1/aoi-interactive-maps/{name_url}/aggregated-results | 
[**get_aoi_interactive_map_handler**](AoiInteractiveMapHandlerApi.md#get_aoi_interactive_map_handler) | **GET** /api/v1/aoi-interactive-maps/{name_url} | 
[**get_aoi_interactive_map_image_handler**](AoiInteractiveMapHandlerApi.md#get_aoi_interactive_map_image_handler) | **GET** /api/v1/aoi-interactive-maps/{name_url}/images/{aoi_image_id} | 



## get_aoi_interactive_map_aggregated_results_handler

> models::PagedResponseTimeSeriesAoiImageAnalysisAggregatedResultsModel get_aoi_interactive_map_aggregated_results_handler(name_url, start, end)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**name_url** | **String** | Aoi name_url field for fetching aoi interactive map details | [required] |
**start** | Option<**String**> | The start time for the search. If not specified, it defaults to the earliest available time in the data. |  |
**end** | Option<**String**> | The end time for the search. If not specified, it defaults to the current time. |  |

### Return type

[**models::PagedResponseTimeSeriesAoiImageAnalysisAggregatedResultsModel**](PagedResponseTimeSeries_AoiImageAnalysisAggregatedResultsModel.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_aoi_interactive_map_handler

> models::AoiInteractiveMapModel get_aoi_interactive_map_handler(name_url)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**name_url** | **String** | Aoi name_url field for fetching aoi interactive map details | [required] |

### Return type

[**models::AoiInteractiveMapModel**](AoiInteractiveMapModel.md)

### Authorization

[third_eye](../README.md#third_eye)

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)


## get_aoi_interactive_map_image_handler

> models::AoiImageForInteractiveMapModel get_aoi_interactive_map_image_handler(name_url, aoi_image_id)


### Parameters


Name | Type | Description  | Required | Notes
------------- | ------------- | ------------- | ------------- | -------------
**name_url** | **String** | Aoi name_url field for fetching aoi interactive map details | [required] |
**aoi_image_id** | **uuid::Uuid** | Aoi image id for fetching aoi image | [required] |

### Return type

[**models::AoiImageForInteractiveMapModel**](AoiImageForInteractiveMapModel.md)

### Authorization

No authorization required

### HTTP request headers

- **Content-Type**: Not defined
- **Accept**: application/json

[[Back to top]](#) [[Back to API list]](../README.md#documentation-for-api-endpoints) [[Back to Model list]](../README.md#documentation-for-models) [[Back to README]](../README.md)

